import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk?version=4.0';

const app = new Gtk.Application({
    application_id: 'org.rayshot.SoakHelper',
    flags: Gio.ApplicationFlags.NON_UNIQUE,
});
app.connect('activate', () => {
    new Gtk.ApplicationWindow({application: app, title: 'rayshot-soak-helper',
        default_width: 640, default_height: 480}).present();
});
app.run([]);
