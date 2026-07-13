return {
    layout = {
        gap = 10,
        margin_top = 30,
        margin_right = 10,
        margin_bottom = 10,
        margin_left = 10,
        default_float_ratio = 0.5,
    },

    decorations = false,

    border_width = 2,
    border_color_focused = 0xff0000ff,
    border_color_unfocused = 0x00ff00ff,

    keybindings = {
        {
            keysym = "Return",
            modifiers = { "super" },
            command = { "spawn", "foot" },
        },
        {
            keysym = "q",
            modifiers = { "super" },
            command = "close",
        },
        {
            keysym = "h",
            modifiers = { "super" },
            command = "focus_left",
        },
        {
            keysym = "j",
            modifiers = { "super" },
            command = "focus_down",
        },
        {
            keysym = "k",
            modifiers = { "super" },
            command = "focus_up",
        },
        {
            keysym = "l",
            modifiers = { "super" },
            command = "focus_right",
        },
        {
            keysym = "Tab",
            modifiers = { "super" },
            command = "focus_next",
        },
        {
            keysym = "Tab",
            modifiers = { "super", "shift" },
            command = "focus_previous",
        },
        {
            keysym = "s",
            modifiers = { "super" },
            command = "toggle_floating",
        },
        {
            keysym = "f",
            modifiers = { "super" },
            command = "toggle_fullscreen",
        },
        {
            keysym = "t",
            modifiers = { "super" },
            command = "tiled",
        },
        {
            keysym = "t",
            modifiers = { "super", "shift" },
            command = "toggle_pseudo_tiled",
        },
        {
            keysym = "r",
            modifiers = { "super", "shift" },
            command = "reload_config",
        },
        {
            keysym = "e",
            modifiers = { "super", "shift" },
            command = "exit_river",
        },
        {
            keysym = "h",
            modifiers = { "super", "shift" },
            command = "move_left",
        },
        {
            keysym = "j",
            modifiers = { "super", "shift" },
            command = "move_down",
        },
        {
            keysym = "k",
            modifiers = { "super", "shift" },
            command = "move_up",
        },
        {
            keysym = "l",
            modifiers = { "super", "shift" },
            command = "move_right",
        },
        {
            keysym = "h",
            modifiers = { "super", "alt" },
            command = { "resize_expand_left" },
        },
        {
            keysym = "j",
            modifiers = { "super", "alt" },
            command = { "resize_expand_down" },
        },
        {
            keysym = "k",
            modifiers = { "super", "alt" },
            command = { "resize_expand_up" },
        },
        {
            keysym = "l",
            modifiers = { "super", "alt" },
            command = { "resize_expand_right" },
        },
        {
            keysym = "h",
            modifiers = { "super", "shift", "alt" },
            command = { "resize_shrink_left" },
        },
        {
            keysym = "j",
            modifiers = { "super", "shift", "alt" },
            command = { "resize_shrink_down" },
        },
        {
            keysym = "k",
            modifiers = { "super", "shift", "alt" },
            command = { "resize_shrink_up" },
        },
        {
            keysym = "l",
            modifiers = { "super", "shift", "alt" },
            command = { "resize_shrink_right" },
        },
    },

    rules = {
        {
            app_id = "mpv",
            mode = "fullscreen",
        },
        {
            app_id = { value = "libreoffice-", match = "prefix" },
            mode = "floating",
        },
        {
            app_id = "org.mozilla.Thunderbird",
            title = { value = "Enter your password.*", match = "regex" },
            mode = "floating",
        },
        {
            app_id = "steam",
            mode = "floating",
            floating_rect = { width = 1280, height = 1080 },
        },
    },
}
