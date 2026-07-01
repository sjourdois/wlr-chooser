//! Build script: generate the English fallback for the no-`i18n` build.

fn main() {
    wlr_i18n::build::generate_fallback("i18n/en/wlr_chooser.ftl");
}
