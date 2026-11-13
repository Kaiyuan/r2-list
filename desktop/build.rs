fn main() {
    let config = slint_build::CompilerConfiguration::default().with_style("cupertino".into());
    slint_build::compile_with_config("ui/main.slint", config).expect("Slint UI compilation failed");

    #[cfg(windows)]
    {
        let mut resources = winres::WindowsResource::new();
        resources.set_icon("app.ico");
        resources
            .compile()
            .expect("failed to embed the Windows application icon");
    }
}
