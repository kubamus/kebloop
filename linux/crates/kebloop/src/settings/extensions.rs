//! Extensions: import a manifest, review what it would run, enable it and edit its preferences.

use std::rc::Rc;

use adw::prelude::*;
use kebloop_services::automation::{
    ExtensionManifest, install_extension, set_extension_enabled, set_extension_preference,
};

use super::logic::PageId;
use super::widgets::{
    DynGroup, Env, Page, action_row, button_row, choose_file, confirm, error_label, group,
    icon_button, note, preferences_page, set_error,
};
use crate::context::Change;
use crate::i18n::{tr, trf};
use kebloop_services::l10n::FormatArg;

/// Manifest size limit (256 KB, as on macOS).
pub const MAX_MANIFEST_BYTES: u64 = 262_144;

/// Parse an extension manifest file's bytes.
pub fn parse_manifest(bytes: &[u8]) -> Result<ExtensionManifest, String> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(tr("Manifest exceeds 256 KB."));
    }
    serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "{} {error}",
            tr("The manifest is not valid JSON for an extension.")
        )
    })
}

/// What the extension would run, for the review.
pub fn command_line(manifest: &ExtensionManifest) -> String {
    std::iter::once(manifest.executable.as_str())
        .chain(manifest.arguments.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the page.
pub fn build(env: &Rc<Env>) -> Page {
    let page = preferences_page();
    page.set_title(&PageId::Extensions.title());
    page.set_description(&PageId::Extensions.subtitle());
    let warning = group("", "");
    warning.add(&note(&tr(
        "Extensions are local subprocesses with your full user privileges, including file and network access. Process isolation is not a sandbox. Install only reviewed code. Preferences are plain text; credential fields and secret export are unsupported.",
    )));
    page.add(&warning);
    let list = Rc::new(DynGroup::new(group(&tr("Extensions"), "")));
    page.add(&list.group);

    let import_group = group("", "");
    let error = error_label();
    {
        let (env, error) = (env.clone(), error.clone());
        import_group.add(&button_row(
            &tr("Import extension manifest…"),
            "document-open-symbolic",
            move || {
                let (env, error) = (env.clone(), error.clone());
                choose_file(
                    &env.clone(),
                    &tr("Import extension manifest…"),
                    move |path| {
                        let outcome = std::fs::metadata(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|meta| {
                                if meta.len() > MAX_MANIFEST_BYTES {
                                    Err(tr("Manifest exceeds 256 KB."))
                                } else {
                                    std::fs::read(&path).map_err(|e| e.to_string())
                                }
                            })
                            .and_then(|bytes| parse_manifest(&bytes))
                            .and_then(|manifest| {
                                let name = manifest.name.clone();
                                env.ctx
                                    .update_settings(|s| {
                                        install_extension(&mut s.extensions, manifest)
                                    })
                                    .map(|()| name)
                                    .map_err(|e| tr(&e.to_string()))
                            });
                        match outcome {
                            Ok(name) => {
                                set_error(&error, None);
                                env.toast(&trf(
                                    "%@ was added disabled. Review it, then enable it.",
                                    &[FormatArg::Str(name)],
                                ));
                            }
                            Err(message) => set_error(&error, Some(&message)),
                        }
                    },
                );
            },
        ));
    }
    import_group.add(&error);
    page.add(&import_group);

    let refresh = {
        let env = env.clone();
        move || {
            let (manifests, saved) = (
                env.ctx.with_settings(|s| s.extensions.clone()),
                env.ctx
                    .config
                    .borrow()
                    .global()
                    .extension_preferences
                    .clone(),
            );
            let signature = format!("{manifests:?}|{saved:?}");
            list.update(&signature, || {
                manifests
                    .iter()
                    .map(|manifest| {
                        extension_row(
                            &env,
                            manifest,
                            saved.get(&manifest.id).cloned().unwrap_or_default(),
                        )
                    })
                    .collect()
            });
        }
    };
    refresh();
    Page::new(&page, refresh)
}

fn extension_row(
    env: &Rc<Env>,
    manifest: &ExtensionManifest,
    saved: std::collections::BTreeMap<String, String>,
) -> gtk::Widget {
    let row = adw::ExpanderRow::builder()
        .title(gtk::glib::markup_escape_text(&manifest.name))
        .subtitle(gtk::glib::markup_escape_text(&command_line(manifest)))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("application-x-addon-symbolic"));
    let toggle = gtk::Switch::builder()
        .valign(gtk::Align::Center)
        .active(manifest.enabled)
        .build();
    {
        let (env, manifest) = (env.clone(), manifest.clone());
        let reverting = Rc::new(std::cell::Cell::new(false));
        toggle.connect_state_set(move |toggle, wanted| {
            if reverting.get() {
                return gtk::glib::Propagation::Proceed;
            }
            let id = manifest.id.clone();
            if !wanted {
                if let Err(message) = env.ctx.edit_settings(|s| {
                    set_extension_enabled(&mut s.extensions, &id, false);
                }) {
                    env.toast(&message);
                    return gtk::glib::Propagation::Stop;
                }
                return gtk::glib::Propagation::Proceed;
            }
            reverting.set(true);
            toggle.set_active(false);
            reverting.set(false);
            let env2 = env.clone();
            confirm(
                &env,
                &trf("Enable %@?", &[FormatArg::Str(manifest.name.clone())]),
                &format!(
                    "{}\n\n{}",
                    tr("Extensions run with your user privileges. Review the executable and arguments before enabling."),
                    command_line(&manifest),
                ),
                &tr("Enable"),
                false,
                move || {
                    if let Err(message) = env2.ctx.edit_settings(|s| {
                        set_extension_enabled(&mut s.extensions, &id, true);
                    }) {
                        env2.toast(&message);
                    }
                },
            );
            gtk::glib::Propagation::Stop
        });
    }
    row.add_suffix(&toggle);

    if !manifest.description.is_empty() {
        row.add_row(&action_row(&manifest.description, ""));
    }
    row.add_row(&action_row(&tr("Executable"), &command_line(manifest)));
    let effective = manifest.effective_preferences(&saved);
    for preference in &manifest.preferences {
        let entry = adw::EntryRow::builder()
            .title(gtk::glib::markup_escape_text(&preference.title))
            .text(effective.get(&preference.id).map_or("", String::as_str))
            .show_apply_button(true)
            .build();
        let (env, manifests, id, key) = (
            env.clone(),
            vec![manifest.clone()],
            manifest.id.clone(),
            preference.id.clone(),
        );
        entry.connect_apply(move |entry| {
            let value = entry.text().to_string();
            let outcome = env.ctx.config.borrow_mut().update_global(|g| {
                set_extension_preference(
                    &manifests,
                    &mut g.extension_preferences,
                    &id,
                    &key,
                    &value,
                );
            });
            match outcome {
                Ok(()) => env.ctx.emit(Change::Settings),
                Err(e) => env.toast(&tr(&e.to_string())),
            }
        });
        row.add_row(&entry);
    }
    let remove = icon_button("user-trash-symbolic", &tr("Remove"));
    {
        let (env, id) = (env.clone(), manifest.id.clone());
        remove.connect_clicked(move |_| {
            let removed = env
                .ctx
                .edit_settings(|s| s.extensions.retain(|m| m.id != id));
            let dropped = env.ctx.config.borrow_mut().update_global(|g| {
                g.extension_preferences.remove(&id);
            });
            match (removed, dropped) {
                (Err(m), _) => env.toast(&m),
                (_, Err(e)) => env.toast(&tr(&e.to_string())),
                _ => env.ctx.emit(Change::Settings),
            }
        });
    }
    row.add_suffix(&remove);
    row.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_parse_and_oversized_or_malformed_ones_are_refused() {
        let json = br#"{"id":"demo","name":"Demo","apiVersion":1,"executable":"/bin/echo","arguments":["x"],"preferences":[{"id":"k","title":"K","defaultValue":"v"}]}"#;
        let manifest = parse_manifest(json).unwrap();
        assert_eq!(manifest.id, "demo");
        assert!(!manifest.enabled);
        assert_eq!(command_line(&manifest), "/bin/echo x");
        assert!(parse_manifest(b"{").is_err());
        let big = vec![b' '; MAX_MANIFEST_BYTES as usize + 1];
        assert!(parse_manifest(&big).is_err());
    }
}
