import Gtk from 'gi://Gtk?version=4.0';
import Gio from 'gi://Gio';
const app = new Gtk.Application({application_id:'app.kebloop.Smoke'});
app.connect('activate', () => {
    const entry = new Gtk.Entry();
    entry.connect('changed', () => print(`ENTRY:${entry.text}`));
    const window = new Gtk.ApplicationWindow({application:app, title:'KebloopPasteSmoke', default_width:400, default_height:120, child:entry});
    window.present(); entry.grab_focus();
});
app.run([]);
