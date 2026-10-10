//! Embeds `roxy-can.ico` into the exe's PE resources, so Explorer, a shortcut,
//! and the taskbar all show it. The window's own title-bar icon is a separate
//! path: `src/icon.rs` decodes the same file at runtime for winit.

fn main() {
    println!("cargo:rerun-if-changed=roxy-can.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("roxy-can.ico");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=没能嵌入 exe 图标: {error}");
        }
    }
}
