fn main() {
    println!("cargo:rerun-if-changed=protocol/river-window-management-v1.xml");
    println!("cargo:rerun-if-changed=protocol/river-xkb-bindings-v1.xml");
}
