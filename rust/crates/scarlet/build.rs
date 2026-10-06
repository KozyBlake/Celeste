fn main() {
    println!("cargo:rerun-if-changed=../../assets/celeste.ico");

    // Build scripts execute for the host, so use Cargo's target variable
    // instead of cfg!(windows); this also works for future cross-compiles.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/celeste.ico")
            .compile()
            .expect("failed to embed the Celeste Windows icon");
    }
}
