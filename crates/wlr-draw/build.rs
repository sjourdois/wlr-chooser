//! Build script: generate the English fallback for the no-`i18n` build from the `en`
//! catalog. Harmless with `i18n` on (the generated file just isn't included).

fn main() {
    wlr_i18n::build::generate_fallback("i18n/en/wlr_draw.ftl");
}
