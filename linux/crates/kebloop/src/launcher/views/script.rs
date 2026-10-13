//! Script command output window: a small separate window with a cancel button and copyable
//! output. The script runs on a worker thread through the shared automation runner.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use kebloop_services::automation::{
    AutomationError, AutomationOutput, AutomationRunner, ScriptCommand, run_script,
};

use crate::context::AppContext;
use crate::i18n::tr;

/// Open the window for `script` and start running it with `extra_arguments`.
pub fn run(ctx: &Rc<AppContext>, script: &ScriptCommand, extra_arguments: Vec<String>) {
    let runner = AutomationRunner::new();
    let cancel = runner.cancel_handle();
    let token = ctx.register_run(cancel.clone());

    let spinner = gtk::Spinner::builder().spinning(true).build();
    let status = gtk::Label::builder()
        .label(tr("Running script…"))
        .xalign(0.0)
        .hexpand(true)
        .build();
    let cancel_button = gtk::Button::with_label(&tr("Cancel Run"));
    let copy_button = gtk::Button::with_label(&tr("Copy"));
    copy_button.set_sensitive(false);
    let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_row.set_margin_top(10);
    status_row.set_margin_bottom(6);
    status_row.set_margin_start(12);
    status_row.set_margin_end(12);
    status_row.append(&spinner);
    status_row.append(&status);
    status_row.append(&copy_button);
    status_row.append(&cancel_button);

    let output = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(10)
        .right_margin(10)
        .top_margin(8)
        .bottom_margin(8)
        .css_classes(["kb-output"])
        .build();
    output.update_property(&[gtk::accessible::Property::Label(&tr("Script output"))]);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&output)
        .vexpand(true)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();
    scroll.add_css_class("card");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&status_row);
    body.append(&scroll);

    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&body));
    let window = adw::Window::builder()
        .title(&script.title)
        .default_width(560)
        .default_height(380)
        .content(&toolbar)
        .build();
    window.set_application(Some(&ctx.app));
    {
        let cancel = cancel.clone();
        cancel_button.connect_clicked(move |_| cancel.cancel());
    }
    {
        let output = output.clone();
        copy_button.connect_clicked(move |button| {
            let buffer = output.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            button.display().clipboard().set_text(&text);
        });
    }
    {
        let cancel = cancel.clone();
        window.connect_close_request(move |_| {
            cancel.cancel();
            glib::Propagation::Proceed
        });
    }
    window.present();

    let (sender, receiver) = async_channel::bounded::<Result<AutomationOutput, AutomationError>>(1);
    let commands = vec![script.clone()];
    let id = script.id.clone();
    let spawned = std::thread::Builder::new()
        .name("kebloop-script".into())
        .spawn(move || {
            let result = run_script(&commands, &id, &extra_arguments, &runner);
            let _ = sender.send_blocking(result);
        });
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let outcome = if spawned.is_ok() {
            receiver.recv().await.ok()
        } else {
            None
        };
        ctx.finish_run(token);
        spinner.set_spinning(false);
        spinner.set_visible(false);
        cancel_button.set_visible(false);
        match outcome {
            Some(Ok(result)) => {
                status.set_text(&tr("Finished"));
                let mut text = result.stdout;
                if !result.stderr.is_empty() {
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str(&result.stderr);
                }
                if text.is_empty() {
                    text = tr("The script printed nothing.");
                }
                output.buffer().set_text(&text);
                copy_button.set_sensitive(true);
            }
            Some(Err(error)) => {
                status.set_text(&error.to_string());
                status.add_css_class("error");
            }
            None => status.set_text(&tr("Could not start the script.")),
        }
    });
}
