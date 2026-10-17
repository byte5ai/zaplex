# dmgbuild settings for the Zaplex installer window. The values reproduce the
# Finder view of the installer shipped up to v1.1.0 (built with create-dmg):
# a 700x500 window without toolbar, status bar or sidebar, the background
# image, 128 pt icons with 16 pt labels, the app on the left and the
# Applications link on the right.
#
# Required defines: -D app=<path to the .app> -D background=<path to the PNG>.
import os.path

application = defines["app"]  # noqa: F821 - provided by dmgbuild
app_name = os.path.basename(application.rstrip("/"))

format = "UDZO"
# APFS: macOS 26.4 has issues mounting HFS+ DMGs; Zaplex targets macOS 12+.
filesystem = "APFS"
files = [application]
symlinks = {"Applications": "/Applications"}

background = defines["background"]  # noqa: F821 - provided by dmgbuild
window_rect = ((10, 208), (700, 500))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = True
show_item_info = False
arrange_by = None
grid_offset = (0, 0)
grid_spacing = 100
label_pos = "bottom"
text_size = 16
icon_size = 128
icon_locations = {
    app_name: (150, 250),
    "Applications": (550, 250),
}
