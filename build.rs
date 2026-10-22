//! Embeds `roxy-can.ico` into the exe's PE resources, so Explorer, a shortcut,
//! and the taskbar all show it.

fn main() {
    println!("cargo:rerun-if-changed=roxy-can.ico");
    println!("cargo:rerun-if-changed=build.rs");
    // rust-embed reads the bundle at compile time; a stale binary must not be
    // built after the frontend changed.
    println!("cargo:rerun-if-changed=web/dist");
    println!("cargo:rerun-if-changed=web/dist/index.html");
    if !std::path::Path::new("web/dist/index.html").exists() {
        panic!(
            "web/dist is missing; run `npm install && npm run build` in web/ before building roxy-can"
        );
    }

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("roxy-can.ico");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=没能嵌入 exe 图标: {error}");
        }
    }
}
