import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';

// With RAYSHOT_SOAK_HELPER_LOG set, every key and focus change is appended to
// that file, so the soak can tell whether a key was left pressed.
const logPath = GLib.getenv('RAYSHOT_SOAK_HELPER_LOG');
function note(line) {
    if (!logPath)
        return;
    const out = Gio.File.new_for_path(logPath).append_to(Gio.FileCreateFlags.NONE, null);
    out.write_all(new TextEncoder().encode(`${line}\n`), null);
    out.close(null);
}

const app = new Gtk.Application({
    application_id: 'org.rayshot.SoakHelper',
    flags: Gio.ApplicationFlags.NON_UNIQUE,
});
app.connect('activate', () => {
    const win = new Gtk.ApplicationWindow({application: app, title: 'rayshot-soak-helper',
        default_width: 640, default_height: 480});
    const keys = new Gtk.EventControllerKey();
    keys.connect('key-pressed', (_c, keyval) => {
        note(`press ${keyval}`);
        return false;
    });
    keys.connect('key-released', (_c, keyval) => note(`release ${keyval}`));
    win.add_controller(keys);
    win.connect('notify::is-active', () => note(win.is_active ? 'focus in' : 'focus out'));
    win.present();
});
app.run([]);
