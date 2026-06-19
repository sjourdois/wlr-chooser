// Recompile (so embedded translations refresh) whenever a catalog changes.
fn main() {
    println!("cargo:rerun-if-changed=i18n");
}
