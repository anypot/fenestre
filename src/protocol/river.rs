// Protocol bindings generated from River's XML protocol definitions.
//
// This module uses wayland-scanner to generate typed Rust code from the
// river-window-management-v1 and river-xkb-bindings-v1 protocol XMLs.

pub mod river_window_management_v1 {
    use wayland_client;
    use wayland_client::protocol::__interfaces::*;

    wayland_scanner::generate_interfaces!("protocol/river-window-management-v1.xml");

    pub mod client {
        use super::*;
        use wayland_client;
        use wayland_client::protocol::wl_surface;

        wayland_scanner::generate_client_code!("protocol/river-window-management-v1.xml");
    }
}

pub mod river_xkb_bindings_v1 {
    use super::river_window_management_v1::*;
    use wayland_client;

    wayland_scanner::generate_interfaces!("protocol/river-xkb-bindings-v1.xml");

    pub mod client {
        use super::*;
        use crate::protocol::river::river_window_management_v1::client::*;
        use wayland_client;

        wayland_scanner::generate_client_code!("protocol/river-xkb-bindings-v1.xml");
    }
}

pub mod river_layer_shell_v1 {
    use super::river_window_management_v1::*;
    use wayland_client;

    wayland_scanner::generate_interfaces!("protocol/river-layer-shell-v1.xml");

    pub mod client {
        use super::*;
        use crate::protocol::river::river_window_management_v1::client::*;
        use wayland_client;

        wayland_scanner::generate_client_code!("protocol/river-layer-shell-v1.xml");
    }
}

pub mod river_input_management_v1 {
    use wayland_client;
    use wayland_client::protocol::__interfaces::*;

    wayland_scanner::generate_interfaces!("protocol/river-input-management-v1.xml");

    pub mod client {
        use super::*;
        use wayland_client;
        use wayland_client::protocol::wl_output;

        wayland_scanner::generate_client_code!("protocol/river-input-management-v1.xml");
    }
}

pub mod river_libinput_config_v1 {
    use super::river_input_management_v1::*;
    use wayland_client;

    wayland_scanner::generate_interfaces!("protocol/river-libinput-config-v1.xml");

    pub mod client {
        use super::*;
        use crate::protocol::river::river_input_management_v1::client::*;
        use wayland_client;

        wayland_scanner::generate_client_code!("protocol/river-libinput-config-v1.xml");
    }
}

pub mod river_xkb_config_v1 {
    use super::river_input_management_v1::*;
    use wayland_client;

    wayland_scanner::generate_interfaces!("protocol/river-xkb-config-v1.xml");

    pub mod client {
        use super::*;
        use crate::protocol::river::river_input_management_v1::client::*;
        use wayland_client;

        wayland_scanner::generate_client_code!("protocol/river-xkb-config-v1.xml");
    }
}
