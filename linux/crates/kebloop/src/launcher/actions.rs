//! Result actions (Ctrl+K): which actions a result offers, and the popover that lists them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use kebloop_platform::apps::DesktopApp;
use kebloop_services::l10n::Localizer;

use super::model::{ResultItem, Target};
use kebloop_services::result_action::ResultActionId;

/// An action on a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultAction {
    /// The primary action (open, run, copy, switch, ...).
    Open,
    /// Show the item in the file manager.
    Reveal,
    /// Copy the item's path.
    CopyPath,
    /// Choose an application to open a file with.
    OpenWith,
    /// Edit a quicklink in settings.
    EditQuicklink,
    /// Pin to favorites (or pin a clipboard item).
    Pin,
    /// Remove from favorites (or unpin a clipboard item).
    Unpin,
    /// Run a script with extra arguments.
    RunWithInput,
    /// Delete a clipboard history entry.
    RemoveClipboardItem,
    /// Hide an application from results.
    HideApp,
    /// Copy a clipboard entry without pasting it.
    Copy,
    /// Paste a clipboard entry without formatting.
    PastePlainText,
    /// Remove every accepted calculation (after confirmation).
    ClearCalculationHistory,
    /// Hide the windows of a running application.
    HideApplication,
    /// Ask a running application to quit.
    QuitApplication,
    /// Terminate a running application (after confirmation).
    ForceQuitApplication,
    /// Quit a running application and start it again.
    RestartApplication,
    /// A `[Desktop Action]` of an application, by action id.
    DesktopAction(String),
    /// An action offered by an extension result, by action id.
    ExtensionAction(String),
}

/// One row of the action menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionEntry {
    /// What it does.
    pub action: ResultAction,
    /// Label.
    pub title: String,
    /// Symbolic icon.
    pub icon: &'static str,
}

/// Facts about the result that are not in the item itself.
#[derive(Debug, Clone, Default)]
pub struct ActionContext {
    /// The result is in the favorites.
    pub pinned: bool,
    /// The quicklink exists in settings (not a synthetic keyword match).
    pub editable_quicklink: bool,
    /// `[Desktop Action]` groups of the application.
    pub desktop_actions: Vec<(String, String)>,
    /// The application has a window.
    pub running: bool,
    /// What the compositor lets Kebloop do to a running application.
    pub controls: crate::platform::AppControls,
    /// The clipboard entry has a plain-text form.
    pub has_plain_text: bool,
}

impl ActionContext {
    /// Desktop actions of `app` (id, localized name).
    pub fn desktop_actions_of(app: &DesktopApp) -> Vec<(String, String)> {
        app.actions
            .iter()
            .map(|a| (a.id.clone(), a.name.clone()))
            .collect()
    }
}

fn icon_of(action: &ResultAction) -> &'static str {
    match action {
        ResultAction::Open => "go-next-symbolic",
        ResultAction::Reveal => "folder-open-symbolic",
        ResultAction::CopyPath => "edit-copy-symbolic",
        ResultAction::OpenWith => "application-x-executable-symbolic",
        ResultAction::EditQuicklink => "document-edit-symbolic",
        ResultAction::Pin => "starred-symbolic",
        ResultAction::Unpin => "non-starred-symbolic",
        ResultAction::RunWithInput => "input-keyboard-symbolic",
        ResultAction::RemoveClipboardItem => "user-trash-symbolic",
        ResultAction::HideApp | ResultAction::HideApplication => "view-conceal-symbolic",
        ResultAction::Copy => "edit-copy-symbolic",
        ResultAction::PastePlainText => "edit-paste-symbolic",
        ResultAction::ClearCalculationHistory => "user-trash-symbolic",
        ResultAction::QuitApplication => "window-close-symbolic",
        ResultAction::ForceQuitApplication => "process-stop-symbolic",
        ResultAction::RestartApplication => "view-refresh-symbolic",
        ResultAction::DesktopAction(_) | ResultAction::ExtensionAction(_) => "go-next-symbolic",
    }
}

/// The actions of `item`, primary action first.
pub fn actions_for(
    item: &ResultItem,
    context: &ActionContext,
    l10n: &Localizer,
) -> Vec<ActionEntry> {
    let entry = |action: ResultAction, title: String| ActionEntry {
        icon: icon_of(&action),
        action,
        title,
    };
    let pin = |pinned: bool| {
        if pinned {
            entry(ResultAction::Unpin, l10n.tr("Unpin from Favorites"))
        } else {
            entry(ResultAction::Pin, l10n.tr("Pin to Favorites"))
        }
    };
    let open = |title: &str| entry(ResultAction::Open, title.to_owned());
    match &item.target {
        Target::Application { .. } => {
            let mut actions = vec![open(&l10n.tr("Open"))];
            for (id, name) in &context.desktop_actions {
                actions.push(entry(ResultAction::DesktopAction(id.clone()), name.clone()));
            }
            if context.running {
                let controls = context.controls;
                let mut control = |id: ResultActionId, action: ResultAction| {
                    actions.push(entry(action, id.title(l10n)));
                };
                if controls.hide {
                    control(
                        ResultActionId::HideApplication,
                        ResultAction::HideApplication,
                    );
                }
                if controls.close {
                    control(
                        ResultActionId::QuitApplication,
                        ResultAction::QuitApplication,
                    );
                }
                if controls.list {
                    control(
                        ResultActionId::ForceQuitApplication,
                        ResultAction::ForceQuitApplication,
                    );
                }
                if controls.close {
                    control(
                        ResultActionId::RestartApplication,
                        ResultAction::RestartApplication,
                    );
                }
            }
            actions.push(entry(
                ResultAction::Reveal,
                l10n.tr("Open Containing Folder"),
            ));
            actions.push(entry(ResultAction::CopyPath, l10n.tr("Copy Path")));
            actions.push(pin(context.pinned));
            actions.push(entry(ResultAction::HideApp, l10n.tr("Hide from Launcher")));
            actions
        }
        Target::File { .. } => vec![
            open(&l10n.tr("Open")),
            entry(ResultAction::Reveal, l10n.tr("Open Containing Folder")),
            entry(ResultAction::CopyPath, l10n.tr("Copy Path")),
            entry(ResultAction::OpenWith, l10n.tr("Open With…")),
            pin(context.pinned),
        ],
        Target::Quicklink { .. } => {
            let mut actions = vec![open(&l10n.tr("Open"))];
            if context.editable_quicklink {
                actions.push(entry(
                    ResultAction::EditQuicklink,
                    l10n.tr("Edit Quicklink…"),
                ));
                actions.push(pin(context.pinned));
            }
            actions
        }
        Target::Command(_) => vec![open(&l10n.tr("Open")), pin(context.pinned)],
        Target::Script(_) => vec![
            open(&l10n.tr("Run")),
            entry(ResultAction::RunWithInput, l10n.tr("Run with Input…")),
        ],
        Target::Clipboard { pinned, .. } => {
            let mut actions = vec![
                open(&l10n.tr("Paste")),
                entry(ResultAction::Copy, ResultActionId::Copy.title(l10n)),
            ];
            if context.has_plain_text {
                actions.push(entry(
                    ResultAction::PastePlainText,
                    ResultActionId::PastePlainText.title(l10n),
                ));
            }
            actions.push(if *pinned {
                entry(ResultAction::Unpin, l10n.tr("Unpin Clipboard Item"))
            } else {
                entry(ResultAction::Pin, l10n.tr("Pin Clipboard Item"))
            });
            actions.push(entry(
                ResultAction::RemoveClipboardItem,
                l10n.tr("Remove Item"),
            ));
            actions
        }
        Target::Calculation { .. } => vec![open(&l10n.tr("Copy Answer"))],
        Target::HistoricalCalculation { .. } => vec![
            open(&l10n.tr("Copy Answer")),
            entry(
                ResultAction::ClearCalculationHistory,
                ResultActionId::ClearCalculationHistory.title(l10n),
            ),
        ],
        Target::WebSearch { .. } => vec![open(&l10n.tr("Search"))],
        Target::Snippet(_) => vec![open(&l10n.tr("Preview"))],
        Target::Profile { active, .. } => {
            if *active {
                Vec::new()
            } else {
                vec![open(&l10n.tr("Switch"))]
            }
        }
        Target::Extension(_) => vec![open(&l10n.tr("Open"))],
        Target::ExtensionResult { .. } => Vec::new(),
    }
}

/// Popover listing the actions of the selected result. It never takes keyboard focus: the
/// launcher forwards Up/Down/Return/Escape to it, so typing keeps working in the search field.
type ChooseCallback = Rc<dyn Fn(ActionEntry)>;

#[derive(Clone)]
pub struct ActionMenu {
    inner: Rc<MenuInner>,
}

struct MenuInner {
    popover: gtk::Popover,
    list: gtk::ListBox,
    title: gtk::Label,
    entries: RefCell<Vec<ActionEntry>>,
    selected: Cell<usize>,
    on_choose: RefCell<Option<ChooseCallback>>,
}

impl ActionMenu {
    /// Create the popover (unparented until [`ActionMenu::attach`]).
    pub fn new() -> Self {
        let title = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["kb-actions-title"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(34)
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .focusable(false)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_width_request(280);
        content.append(&title);
        content.append(&list);
        let popover = gtk::Popover::builder()
            .child(&content)
            .autohide(false)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .css_classes(["kb-actions"])
            .focusable(false)
            .build();
        let inner = Rc::new(MenuInner {
            popover,
            list,
            title,
            entries: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            on_choose: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        inner.list.connect_row_activated(move |_, row| {
            let Some(inner) = weak.upgrade() else { return };
            let index = usize::try_from(row.index()).unwrap_or(usize::MAX);
            ActionMenu { inner }.choose(index);
        });
        Self { inner }
    }

    /// Parent the popover to `widget` (the Actions button).
    pub fn attach(&self, widget: &impl IsA<gtk::Widget>) {
        self.inner.popover.set_parent(widget);
        self.inner.popover.set_halign(gtk::Align::End);
    }

    /// Whether the menu is showing.
    pub fn is_open(&self) -> bool {
        self.inner.popover.is_visible()
    }

    /// Call `f` whenever the menu opens or closes.
    pub fn connect_open_changed(&self, f: impl Fn(bool) + 'static) {
        self.inner
            .popover
            .connect_visible_notify(move |popover| f(popover.is_visible()));
    }

    /// Show `entries` for a result called `title`.
    pub fn show(
        &self,
        title: &str,
        entries: Vec<ActionEntry>,
        on_choose: impl Fn(ActionEntry) + 'static,
    ) {
        let inner = &self.inner;
        while let Some(child) = inner.list.first_child() {
            inner.list.remove(&child);
        }
        for entry in &entries {
            let icon = gtk::Image::from_icon_name(entry.icon);
            icon.set_pixel_size(16);
            let label = gtk::Label::builder()
                .label(&entry.title)
                .xalign(0.0)
                .hexpand(true)
                .build();
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row_box.append(&icon);
            row_box.append(&label);
            let row = gtk::ListBoxRow::builder()
                .child(&row_box)
                .css_classes(["kb-action"])
                .focusable(false)
                .build();
            inner.list.append(&row);
        }
        inner.title.set_text(title);
        *inner.entries.borrow_mut() = entries;
        *inner.on_choose.borrow_mut() = Some(Rc::new(on_choose));
        inner.selected.set(0);
        self.update_selection();
        inner.popover.popup();
    }

    /// Hide the menu.
    pub fn close(&self) {
        self.inner.popover.popdown();
    }

    /// Move the selection by `delta`, clamped.
    pub fn move_selection(&self, delta: isize) {
        let count = self.inner.entries.borrow().len();
        if count == 0 {
            return;
        }
        let next = self
            .inner
            .selected
            .get()
            .saturating_add_signed(delta)
            .min(count - 1);
        self.inner.selected.set(next);
        self.update_selection();
    }

    /// Run the selected action and close.
    pub fn activate_selected(&self) {
        self.choose(self.inner.selected.get());
    }

    fn choose(&self, index: usize) {
        let entry = self.inner.entries.borrow().get(index).cloned();
        let callback = self.inner.on_choose.borrow().clone();
        self.close();
        if let (Some(entry), Some(callback)) = (entry, callback) {
            callback(entry);
        }
    }

    fn update_selection(&self) {
        let selected = self.inner.selected.get();
        let mut child = self.inner.list.first_child();
        let mut position = 0;
        while let Some(row) = child {
            if position == selected {
                row.add_css_class("kb-selected");
            } else {
                row.remove_css_class("kb-selected");
            }
            position += 1;
            child = row.next_sibling();
        }
    }
}

impl Default for ActionMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MenuInner {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::model::IconSpec;
    use kebloop_services::quicklinks::Quicklink;

    fn item(target: Target) -> ResultItem {
        ResultItem {
            id: "x".into(),
            target,
            title: "t".into(),
            subtitle: String::new(),
            icon: IconSpec::Symbolic("x"),
            action: "Open".into(),
        }
    }

    fn titles(actions: &[ActionEntry]) -> Vec<&str> {
        actions.iter().map(|a| a.title.as_str()).collect()
    }

    #[test]
    fn application_actions_include_desktop_actions_and_hide() {
        let l10n = Localizer::english();
        let context = ActionContext {
            desktop_actions: vec![("new-window".into(), "New Window".into())],
            ..ActionContext::default()
        };
        let actions = actions_for(
            &item(Target::Application {
                desktop_id: "a.desktop".into(),
                path: "/a.desktop".into(),
            }),
            &context,
            &l10n,
        );
        assert_eq!(
            titles(&actions),
            [
                "Open",
                "New Window",
                "Open Containing Folder",
                "Copy Path",
                "Pin to Favorites",
                "Hide from Launcher"
            ]
        );
    }

    #[test]
    fn pinned_results_offer_unpin() {
        let l10n = Localizer::english();
        let context = ActionContext {
            pinned: true,
            ..ActionContext::default()
        };
        let actions = actions_for(
            &item(Target::File {
                path: "/x".into(),
                is_dir: false,
            }),
            &context,
            &l10n,
        );
        assert!(titles(&actions).contains(&"Unpin from Favorites"));
        assert!(!titles(&actions).contains(&"Pin to Favorites"));
    }

    #[test]
    fn keyword_quicklinks_cannot_be_edited_or_pinned() {
        let l10n = Localizer::english();
        let link = Quicklink {
            id: "web:ddg".into(),
            name: "DuckDuckGo".into(),
            target: "https://duckduckgo.com/?q={query}".into(),
            keyword: "web".into(),
        };
        let target = Target::Quicklink {
            link,
            argument: "x".into(),
        };
        let synthetic = actions_for(&item(target.clone()), &ActionContext::default(), &l10n);
        assert_eq!(titles(&synthetic), ["Open"]);
        let editable = actions_for(
            &item(target),
            &ActionContext {
                editable_quicklink: true,
                ..ActionContext::default()
            },
            &l10n,
        );
        assert_eq!(
            titles(&editable),
            ["Open", "Edit Quicklink…", "Pin to Favorites"]
        );
    }

    #[test]
    fn clipboard_actions() {
        let l10n = Localizer::english();
        let clipboard = item(Target::Clipboard {
            entry_id: "1".into(),
            pinned: true,
        });
        let context = ActionContext {
            has_plain_text: true,
            ..ActionContext::default()
        };
        let actions = actions_for(&clipboard, &context, &l10n);
        assert_eq!(
            titles(&actions),
            [
                "Paste",
                "Copy",
                "Paste as Plain Text",
                "Unpin Clipboard Item",
                "Remove Item"
            ]
        );
        assert_eq!(actions[4].action, ResultAction::RemoveClipboardItem);
        let image = actions_for(&clipboard, &ActionContext::default(), &l10n);
        assert_eq!(
            titles(&image),
            ["Paste", "Copy", "Unpin Clipboard Item", "Remove Item"],
            "an entry without text cannot be pasted as plain text"
        );
    }

    #[test]
    fn running_applications_offer_the_controls_the_compositor_supports() {
        let l10n = Localizer::english();
        let app = item(Target::Application {
            desktop_id: "a.desktop".into(),
            path: "/a.desktop".into(),
        });
        let niri = ActionContext {
            running: true,
            controls: crate::platform::AppControls {
                hide: false,
                close: true,
                list: true,
            },
            ..ActionContext::default()
        };
        assert_eq!(
            titles(&actions_for(&app, &niri, &l10n)),
            [
                "Open",
                "Quit Application",
                "Force Quit Application…",
                "Restart Application",
                "Open Containing Folder",
                "Copy Path",
                "Pin to Favorites",
                "Hide from Launcher"
            ]
        );
        let sway = ActionContext {
            controls: crate::platform::AppControls {
                hide: true,
                close: true,
                list: true,
            },
            ..niri.clone()
        };
        assert_eq!(
            titles(&actions_for(&app, &sway, &l10n))[1],
            "Hide Application"
        );
        let stopped = ActionContext {
            running: false,
            ..sway
        };
        assert!(!titles(&actions_for(&app, &stopped, &l10n)).contains(&"Quit Application"));
    }

    #[test]
    fn calculator_history_rows_offer_copy_and_clear() {
        let l10n = Localizer::english();
        let row = item(Target::HistoricalCalculation {
            result: kebloop_core::CalculationResult {
                expression: "1+1".into(),
                display: "2".into(),
                clipboard: "2".into(),
            },
        });
        let actions = actions_for(&row, &ActionContext::default(), &l10n);
        assert_eq!(
            titles(&actions),
            ["Copy Answer", "Clear Calculator History…"]
        );
        assert_eq!(actions[1].action, ResultAction::ClearCalculationHistory);
    }
}
