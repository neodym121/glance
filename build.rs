fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile the Slint UI");

    // App and tray icons plus the manifest (per-monitor DPI awareness v2).
    println!("cargo:rerun-if-changed=assets");
    embed_resource::compile("assets/glance.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile Windows resources");
}
