/// The window class a build reports to the window manager, so a compositor
/// rule or a launcher's `StartupWMClass` can tell the builds apart. The
/// dev install's `.desktop` file names `Hyprlayer-Dev`.
pub fn for_build(identifier: &str, debug: bool) -> &'static str {
    if debug {
        "Hyprlayer-Debug"
    } else if identifier == hyprlayer_server::APP_IDENTIFIER {
        "Hyprlayer"
    } else {
        "Hyprlayer-Dev"
    }
}

/// GTK sends the program name as the Wayland app id and the X11 `WM_CLASS`,
/// and reads it once when it initialises, so this must run before the builder.
pub fn apply(identifier: &str) {
    gtk::glib::set_prgname(Some(for_build(identifier, cfg!(debug_assertions))));
}

#[cfg(test)]
mod tests {
    use super::for_build;

    #[test]
    fn each_build_kind_has_its_own_class() {
        assert_eq!(for_build("ai.brightblock.hyprlayer", false), "Hyprlayer");
        assert_eq!(
            for_build("ai.brightblock.hyprlayer.main", false),
            "Hyprlayer-Dev"
        );
        assert_eq!(
            for_build("ai.brightblock.hyprlayer.dev", true),
            "Hyprlayer-Debug"
        );
        assert_eq!(
            for_build("ai.brightblock.hyprlayer.verify-1", true),
            "Hyprlayer-Debug"
        );
    }
}
