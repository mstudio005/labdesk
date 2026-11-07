// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::*;

fn store() -> Store { Store::open_in_memory().unwrap() }
const CBC: &str = r#"{"tests":[{"code":"CBC","name":"CBC","department":"Haematology","sample":"Blood","price_paise":35000,"config":{"parameters":[{"id":"hb","name":"Hb","unit":"g/dL","low":13,"high":17}]}}]}"#;

#[test]
fn first_admin_then_sign_in() {
    let mut s = store();
    assert!(!s.has_users().unwrap());
    let a = s.create_first_admin("  Mutaib ", "Syed Mutaib", "secret1").unwrap();
    assert_eq!(a.username, "mutaib");
    assert!(s.create_first_admin("x", "y", "secret1").is_err(), "only once");
    assert_eq!(s.authenticate("MUTAIB", "secret1").unwrap().role, Role::Admin);
    assert!(s.authenticate("mutaib", "wrong").is_err());
    assert!(s.authenticate("nobody", "secret1").is_err());
    let e1 = s.authenticate("mutaib", "wrong").unwrap_err().to_string();
    let e2 = s.authenticate("nobody", "x").unwrap_err().to_string();
    assert_eq!(e1, e2, "do not reveal which part was wrong");
}

#[test]
fn admin_manages_accounts() {
    let mut s = store();
    let admin = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    let act = admin.actor();
    let d = s.create_user(&act, "reception1", "Front Desk", Role::Reception, "pass123").unwrap();
    assert!(s.create_user(&act, "RECEPTION1", "Dup", Role::Reception, "pass123").is_err(), "unique, case-insensitive");
    assert!(s.create_user(&act, "a b", "Bad", Role::Reception, "pass123").is_err());
    assert!(s.create_user(&act, "shortpw", "X", Role::Reception, "123").is_err());
    // non-admins cannot manage accounts
    let desk = s.authenticate("reception1", "pass123").unwrap();
    assert!(matches!(s.create_user(&desk.actor(), "evil", "Evil", Role::Admin, "pass123"), Err(Error::Forbidden(_))));
    assert!(matches!(s.reset_password(&desk.actor(), admin.id, "hacked1"), Err(Error::Forbidden(_))));
    // reset + change password
    s.reset_password(&act, d.id, "newpass1").unwrap();
    assert!(s.authenticate("reception1", "pass123").is_err());
    s.authenticate("reception1", "newpass1").unwrap();
    assert!(s.change_password(d.id, "bad", "another1").is_err());
    s.change_password(d.id, "newpass1", "another1").unwrap();
    s.authenticate("reception1", "another1").unwrap();
    // switching off blocks sign-in
    s.update_user(&act, d.id, "Front Desk", Role::Reception, false).unwrap();
    assert!(s.authenticate("reception1", "another1").unwrap_err().to_string().contains("switched off"));
    assert_eq!(s.users().unwrap().len(), 2);
}

#[test]
fn last_admin_is_protected() {
    let mut s = store();
    let admin = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    let act = admin.actor();
    assert!(s.update_user(&act, admin.id, "Boss", Role::Reception, true).is_err());
    assert!(s.update_user(&act, admin.id, "Boss", Role::Admin, false).is_err());
    let second = s.create_user(&act, "two", "Two", Role::Admin, "secret2").unwrap();
    s.update_user(&act, admin.id, "Boss", Role::Reception, true).unwrap();
    assert!(s.update_user(&second.actor(), second.id, "Two", Role::Admin, false).is_err());
}

#[test]
fn edit_tests_by_hand() {
    let mut s = store();
    let admin = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    let act = admin.actor();
    s.import_catalogue(&act, CBC).unwrap();
    let mut t = s.tests().unwrap().remove(0);
    t.price_paise = 40000;
    t.config.parameters[0].high = Some(17.5);
    t.config.parameters.push(Parameter { name: "RBC".into(), field: "Numeric".into(), unit: "m/uL".into(), ..Default::default() });
    let saved = s.save_test(&act, &t).unwrap();
    assert_eq!(saved.price_paise, 40000);
    assert_eq!(saved.config.parameters.len(), 2);
    assert!(!saved.config.parameters[1].id.is_empty());
    // reversed range, duplicate code and non-admin are all refused with a readable message
    let mut bad = saved.clone(); bad.config.parameters[0].low = Some(20.0);
    assert!(s.save_test(&act, &bad).unwrap_err().to_string().contains("reversed"));
    let mut new = saved.clone(); new.id = 0;
    assert!(s.save_test(&act, &new).unwrap_err().to_string().contains("already uses"));
    new.code = "cbc2".into(); new.config.parameters.clear();
    let created = s.save_test(&act, &new).unwrap();
    assert_eq!(created.code, "CBC2");
    let desk = s.create_user(&act, "desk1", "Desk", Role::Reception, "secret1").unwrap();
    assert!(matches!(s.save_test(&desk.actor(), &saved), Err(Error::Forbidden(_))));
    // an order registered before an edit keeps the old test definition
    let o = s.register(&act, NewOrder { patient: NewPatient { first_name: "A".into(), age: 30.0, age_type: "Year".into(), gender: "Male".into(), address: "x".into(), ..Default::default() },
        items: vec![OrderItem { test_id: saved.id, price_paise: None, discount_paise: 0 }], ..Default::default() }).unwrap();
    let mut changed = saved.clone(); changed.config.parameters[0].high = Some(99.0);
    s.save_test(&act, &changed).unwrap();
    assert_eq!(s.order(o.id).unwrap().tests[0].config.parameters[0].high, Some(17.5));
}

#[test]
fn admin_passwords_are_private_and_accounts_can_be_removed() {
    let mut s = store();
    let a = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    let act = a.actor();
    let b = s.create_user(&act, "second", "Second", Role::Admin, "secret2").unwrap();
    let d = s.create_user(&act, "desk1", "Desk", Role::Reception, "secret3").unwrap();
    // an Admin cannot set another Admin's password, only the owner can (knowing the current one)
    assert!(s.reset_password(&act, b.id, "hacked12").unwrap_err().to_string().contains("only be changed by that Admin"));
    s.change_password(b.id, "secret2", "better22").unwrap();
    // re-typing the password
    s.confirm_password(a.id, "secret1").unwrap();
    assert!(s.confirm_password(a.id, "nope").is_err());
    // removing: not yourself, not as non-admin, otherwise fine
    assert!(s.delete_user(&act, a.id, a.id).is_err());
    assert!(matches!(s.delete_user(&d.actor(), d.id, b.id), Err(Error::Forbidden(_))));
    s.delete_user(&act, a.id, d.id).unwrap();
    assert!(s.authenticate("desk1", "secret3").is_err());
    assert_eq!(s.users().unwrap().len(), 2);
    // the other Admin can remove the first; the last one can not be removed
    s.delete_user(&b.actor(), b.id, a.id).unwrap();
    assert!(s.delete_user(&b.actor(), a.id, b.id).is_err());
}

#[test]
fn choice_lines_need_options() {
    let mut s = store();
    let a = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    let act = a.actor();
    s.import_catalogue(&act, CBC).unwrap();
    let mut t = s.tests().unwrap().remove(0);
    t.config.parameters.push(Parameter { name: "Colour".into(), field: "Dropdown".into(), ..Default::default() });
    assert!(s.save_test(&act, &t).unwrap_err().to_string().contains("choice list"));
    t.config.parameters[1].options = vec!["Pale yellow".into(), "pale yellow".into(), " Amber ".into(), "".into()];
    let saved = s.save_test(&act, &t).unwrap();
    assert_eq!(saved.config.parameters[1].options, vec!["Pale yellow", "Amber"]);
}

#[test]
fn forgotten_admin_password_can_be_recovered_with_the_code() {
    let mut s = store();
    let a = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    assert!(!s.has_recovery_code().unwrap());
    let code = s.new_recovery_code(&a.actor()).unwrap();
    assert_eq!(code.len(), 23);
    let desk = s.create_user(&a.actor(), "desk1", "Desk", Role::Reception, "secret3").unwrap();
    assert!(matches!(s.new_recovery_code(&desk.actor()), Err(Error::Forbidden(_))));
    // wrong code, wrong user, or a non-admin user all get the same answer
    assert!(s.recover_admin("boss", "AAAAA-AAAAA-AAAAA-AAAAA", "newpass1").is_err());
    assert!(s.recover_admin("nobody", &code, "newpass1").is_err());
    assert!(s.recover_admin("desk1", &code, "newpass1").is_err());
    // the right code (typed in lower case, without dashes) works once
    let typed = code.replace('-', "").to_lowercase();
    let (user, fresh) = s.recover_admin(" Boss ", &typed, "newpass1").unwrap();
    assert_eq!(user.username, "boss");
    s.authenticate("boss", "newpass1").unwrap();
    assert!(s.authenticate("boss", "secret1").is_err());
    assert!(s.recover_admin("boss", &code, "again123").is_err(), "the used code is dead");
    assert_ne!(fresh, code);
    s.recover_admin("boss", &fresh, "again123").unwrap();
}

#[test]
fn the_recovery_code_shows_the_admin_username() {
    let mut s = store();
    let a = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    s.create_user(&a.actor(), "desk1", "Desk", Role::Reception, "secret3").unwrap();
    assert!(s.admin_usernames("AAAAA-AAAAA-AAAAA-AAAAA").is_err(), "no code has been made yet");
    let code = s.new_recovery_code(&a.actor()).unwrap();
    assert!(s.admin_usernames("AAAAA-AAAAA-AAAAA-AAAAA").is_err());
    let typed = code.replace('-', "").to_lowercase();
    assert_eq!(s.admin_usernames(&typed).unwrap(), vec!["boss".to_string()], "only Admins are shown");
    assert_eq!(s.admin_usernames(&code).unwrap(), vec!["boss".to_string()], "looking does not use the code up");
    s.recover_admin("boss", &code, "newpass1").unwrap();
}

#[test]
fn standard_test_list_imports_cleanly() {
    let mut s = store();
    let a = s.create_first_admin("boss", "Boss", "secret1").unwrap();
    let text = include_str!("../../labdesk-desktop/assets/standard-tests.json");
    assert_eq!(s.import_catalogue(&a.actor(), text).unwrap(), 288);
    assert_eq!(s.tests().unwrap().len(), 288);
    assert_eq!(s.import_catalogue(&a.actor(), text).unwrap(), 288, "importing twice does not duplicate");
    assert_eq!(s.tests().unwrap().len(), 288);
}
