/* A real Flatpak caller. Its sandbox has no direct access to the selected file.
 */
#include <gio/gio.h>
#include <unistd.h>

static GMainLoop *loop;
static gboolean granted;

static void response(GDBusConnection *bus, const char *sender, const char *path,
                     const char *iface, const char *member, GVariant *args,
                     gpointer data) {
  guint code;
  g_autoptr(GVariant) results = NULL;
  g_variant_get(args, "(u@a{sv})", &code, &results);
  g_auto(GStrv) uris = NULL;
  if (code == 0 && g_variant_lookup(results, "uris", "^as", &uris) && uris[0]) {
    g_autoptr(GFile) file = g_file_new_for_uri(uris[0]);
    g_autofree char *text = NULL;
    g_autofree char *prefix =
        g_strdup_printf("file://%s/doc/", g_get_user_runtime_dir());
    granted = g_str_has_prefix(uris[0], prefix) &&
              g_file_load_contents(file, NULL, &text, NULL, NULL, NULL) &&
              g_strcmp0(text, "portal test\n") == 0;
    g_print("Document URI %s, readable %d\n", uris[0], granted);
  }
  g_main_loop_quit(loop);
}

int main(void) {
  g_autofree char *folder =
      g_build_filename(g_get_user_runtime_dir(), "mundane-portal-test", NULL);
  g_autofree char *original = g_build_filename(folder, "example.txt", NULL);
  g_assert(access(original, R_OK) < 0);
  g_autoptr(GError) error = NULL;
  g_autoptr(GDBusConnection) bus =
      g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
  if (!bus)
    g_error("%s", error->message);
  const char *portal = "org.freedesktop.portal.Desktop";
  loop = g_main_loop_new(NULL, FALSE);
  g_dbus_connection_signal_subscribe(
      bus, portal, "org.freedesktop.portal.Request", "Response", NULL, NULL,
      G_DBUS_SIGNAL_FLAGS_NONE, response, NULL, NULL);
  GVariantBuilder options;
  g_variant_builder_init(&options, G_VARIANT_TYPE_VARDICT);
  g_variant_builder_add(&options, "{sv}", "current_folder",
                        g_variant_new_bytestring(folder));
  g_autoptr(GVariant) handle = g_dbus_connection_call_sync(
      bus, portal, "/org/freedesktop/portal/desktop",
      "org.freedesktop.portal.FileChooser", "OpenFile",
      g_variant_new("(ssa{sv})", "", "Sandbox file", &options),
      G_VARIANT_TYPE("(o)"), G_DBUS_CALL_FLAGS_NONE, 10000, NULL, &error);
  if (!handle)
    g_error("%s", error->message);
  g_main_loop_run(loop);
  g_main_loop_unref(loop);
  return granted ? 0 : 1;
}
