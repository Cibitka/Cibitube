use crate::i18n::tr;
use std::process::Command;

#[allow(dead_code)]
pub struct DonateConfig {
    pub boosty_url: &'static str,
    pub ton_usdt_wallet: &'static str,
    pub ton_deeplink: &'static str,
}

impl Default for DonateConfig {
    fn default() -> Self {
        Self {
            boosty_url: "https://boosty.to/zch_cibitka/donate",
            ton_usdt_wallet: "UQDTl3ZXiUqMwQG-pWw3W2OvphGb9jRUGv6Wmsn1ZBOHV8NO",
            ton_deeplink: "ton://transfer/UQDTl3ZXiUqMwQG-pWw3W2OvphGb9jRUGv6Wmsn1ZBOHV8NO",
        }
    }
}

#[allow(dead_code)]
pub fn donate_labels(lang_code: &str) -> (&'static str, &'static str, &'static str) {
    let s = tr(lang_code);
    (s.donate_title, s.donate_boosty_btn, s.donate_copy_ton_btn)
}

pub fn open_boosty_donate() {
    open_donation_url("https://boosty.to/zch_cibitka/donate");
}

pub fn open_donation_url(url: &str) {
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd").args(["/C", "start", url]).spawn();
}
