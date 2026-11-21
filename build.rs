fn main() {
    println!("cargo:rerun-if-changed=assets/spotify.ico");

    let mut resources = winres::WindowsResource::new();
    resources.set_icon("assets/spotify.ico");
    resources.set("ProductName", "SpotiFast");
    resources.set("FileDescription", "SpotiFast Music Player");
    resources.set("InternalName", "SpotiFast");
    resources.set("OriginalFilename", "SpotiFast.exe");
    resources
        .compile()
        .expect("failed to embed the Spotify app icon");
}
