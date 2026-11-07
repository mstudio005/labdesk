// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::*;

fn admin() -> Actor { Actor::new("Dr Admin", Role::Admin) }

#[test]
fn addresses_are_checked_and_presets_fill_the_server() {
    assert!(valid_address("lab@example.com") && valid_address(" a.b+c@mail.co.in "));
    for bad in ["", "lab", "lab@", "@x.com", "a@b", "a b@c.com", "a@@b.com", "a@b..com", "a@b.com,c@d.com", "<a@b.com>"] { assert!(!valid_address(bad), "{bad}"); }
    assert_eq!(email_preset("proton"), Some(("smtp.protonmail.ch", 587)));
    assert_eq!(email_preset("gmail"), Some(("smtp.gmail.com", 587)));
    assert_eq!(email_preset("custom"), None);
}

#[test]
fn email_is_off_by_default_only_the_admin_saves_it_and_a_provider_fills_the_server() {
    let mut s = Store::open_in_memory().unwrap();
    let d = s.email_settings().unwrap();
    assert!(!d.enabled && !d.ready());
    let want = EmailSettings { enabled: true, provider: "gmail".into(), host: "ignored.example".into(), port: 25, username: "lab@gmail.com".into(), password: "abcd efgh ijkl mnop".into(), from_name: " City Lab ".into() };
    assert!(s.save_email_settings(&Actor::new("Front", Role::Reception), &want).is_err());
    s.save_email_settings(&admin(), &want).unwrap();
    let got = s.email_settings().unwrap();
    assert_eq!((got.host.as_str(), got.port, got.from_name.as_str()), ("smtp.gmail.com", 587, "City Lab"));
    assert!(got.ready());
    // missing pieces are refused only when turning it on
    let mut bad = want.clone(); bad.username = "not an address".into();
    assert!(s.save_email_settings(&admin(), &bad).is_err());
    bad.password.clear(); bad.username = "lab@gmail.com".into();
    assert!(s.save_email_settings(&admin(), &bad).is_err());
    bad.enabled = false;
    s.save_email_settings(&admin(), &bad).unwrap();
    assert!(!s.email_settings().unwrap().ready());
    // custom server keeps what was typed
    let c = EmailSettings { enabled: true, provider: "anything".into(), host: " mail.lab.in ".into(), port: 465, username: "a@lab.in".into(), password: "x".into(), from_name: String::new() };
    s.save_email_settings(&admin(), &c).unwrap();
    let got = s.email_settings().unwrap();
    assert_eq!((got.provider.as_str(), got.host.as_str(), got.port), ("custom", "mail.lab.in", 465));
}
