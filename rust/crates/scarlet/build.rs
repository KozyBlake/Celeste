fn main() {
    println!("cargo:rerun-if-changed=../../assets/celeste.ico");

    // Build scripts execute for the host, so use Cargo's target variable
    // instead of cfg!(windows); this also works for future cross-compiles.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../../assets/celeste.ico")
            .set(
                "FileDescription",
                "A VRChat moderation logger based on Scarlet, but coded in Rust.",
            )
            .set("ProductName", "Celeste")
            .set("InternalName", "Celeste")
            .set("OriginalFilename", "celeste.exe")
            .set("CompanyName", "KozyBlake")
            .set(
                "Comments",
                "VRChat moderation, logging, and group operations",
            );
        resource
            .compile()
            .expect("failed to embed the Celeste Windows icon");
    }
}
