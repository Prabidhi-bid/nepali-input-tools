fn main() {
    // Drive COM DLL exports from the .def file: marks the four helpers PRIVATE
    // and keeps DllMain out of the export table.
    #[cfg(windows)]
    {
        let def = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("xlit_tsf.def");
        println!("cargo:rerun-if-changed={}", def.display());
        println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
    }
}
