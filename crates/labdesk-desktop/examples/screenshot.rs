// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Dev tool: renders the real UI headlessly (software renderer) through a scripted workflow and
//! writes PNGs. The clock is under our control so animations are finished before each picture.
//! Usage: cargo run -p labdesk-desktop --example screenshot -- OUT_DIR [LETTERHEAD.png]
use labdesk_core::{Actor, Role, Store};
use labdesk_desktop::{app, AppWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Model, PhysicalSize, PlatformError, Rgb8Pixel, SharedPixelBuffer};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

thread_local! { static NOW: Cell<u64> = const { Cell::new(0) }; }
struct P(Rc<MinimalSoftwareWindow>);
impl Platform for P {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> { Ok(self.0.clone()) }
    fn duration_since_start(&self) -> Duration { Duration::from_millis(NOW.with(|n| n.get())) }
}

const UX: f32 = 835.0; // where the first line's answer-type dropdown sits in the Urine editor (updated by eye)
const UY: f32 = 337.0;
const UNITX: f32 = 975.0;
const UNITY: f32 = 475.0;
const W: u32 = 1360;
const H: u32 = 820;

fn advance(ms: u64) {
    for _ in 0..(ms / 50).max(1) { NOW.with(|n| n.set(n.get() + 50)); slint::platform::update_timers_and_animations(); }
}
fn snap(win: &Rc<MinimalSoftwareWindow>, out: &std::path::Path, name: &str, settle_ms: u64) {
    // draw once so animations start, then let them finish
    let mut warm = SharedPixelBuffer::<Rgb8Pixel>::new(W, H);
    win.request_redraw();
    win.draw_if_needed(|r| { r.render(warm.make_mut_slice(), W as usize); });
    advance(settle_ms);
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(W, H);
    win.request_redraw();
    win.draw_if_needed(|r| { r.render(buf.make_mut_slice(), W as usize); });
    let path = out.join(name);
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path).unwrap()), W, H);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(buf.as_bytes()).unwrap();
    println!("wrote {}", path.display());
}

fn click(ui: &AppWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    ui.window().dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
    ui.window().dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
    advance(100);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let letterhead = args.next();
    std::fs::create_dir_all(&out).unwrap();
    let win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(P(win.clone()))).unwrap();
    win.set_size(PhysicalSize::new(W, H));
    let mut store = Store::open_in_memory().unwrap();
    store.accept_terms().unwrap();
    store.import_catalogue(&Actor::new("Setup", Role::Admin), app::DEMO_CATALOGUE).unwrap();
    let ui = AppWindow::new().unwrap();
    ui.show().unwrap();
    let opts = app::Options { docs_dir: out.join("docs"), open_files: false, ..app::Options::new() };
    let ctl = app::wire(&ui, store, opts);

    // Before anything else: the Terms of Use and Privacy.
    ui.set_terms_needed(true); ui.set_terms_ok(true);
    snap(&win, &out, "00-terms.png", 700);
    ui.set_terms_needed(false); ui.set_terms_ok(false);

    // First start: the lab creates its Admin account.
    snap(&win, &out, "01-first-run.png", 900);
    ui.set_a_name("Syed Mutaib".into()); ui.set_a_user("mutaib".into()); ui.set_a_pass("secret1".into()); ui.set_a_pass2("secret2".into());
    ui.invoke_create_first_admin();
    snap(&win, &out, "01b-first-run-mismatch.png", 500);
    ui.set_a_pass2("secret1".into());
    ui.invoke_create_first_admin();
    println!("signed in: {} as {} ({})", ui.get_signed_in(), ui.get_user_name(), ui.get_user_role());
    let code = ui.get_rc_code().to_string();
    println!("recovery code shown: {} ({})", ui.get_rc_shown(), code);
    snap(&win, &out, "01c-recovery-code.png", 600);
    ui.invoke_rc_done();
    snap(&win, &out, "02-home.png", 1500);

    // Tests tab: open a test, change a limit, save
    ui.set_s_tab(2); ui.invoke_navigate(3);
    ui.invoke_te_open(1);
    snap(&win, &out, "03-tests.png", 900);
    ui.invoke_fp_pick(0); advance(200);
    snap(&win, &out, "03a-field-panel.png", 500);
    ui.set_fp_to("18".into()); ui.invoke_fp_submit();
    ui.invoke_te_save();
    println!("test save toast: {} (bad={})", ui.get_toast_text(), ui.get_toast_bad());
    println!("fields: {}", ui.get_frows().row_count());
    // age ranges box
    ui.invoke_fp_pick(0); ui.invoke_ar_open(true); advance(400);
    snap(&win, &out, "03g-age-ranges.png", 500);
    ui.invoke_ar_edit(0, 7, "Female".into()); ui.invoke_ar_edit(0, 0, "1".into()); ui.invoke_ar_edit(0, 1, "12".into()); ui.invoke_ar_edit(0, 2, "11".into()); ui.invoke_ar_edit(0, 3, "15".into());
    ui.invoke_ar_done(); ui.invoke_fp_submit(); ui.invoke_te_save();
    println!("after age range: {}", ui.get_toast_text());
    ui.invoke_te_open_text(0); advance(300);
    snap(&win, &out, "03h-comment-box.png", 400);
    ui.invoke_ci_cancel();
    ui.invoke_te_preview(); advance(300);
    println!("preview toast: {}", ui.get_toast_text());
    ui.invoke_te_back();
    snap(&win, &out, "03i-test-list.png", 400);
    ui.invoke_te_new();
    ui.invoke_te_save();
    snap(&win, &out, "04-test-errors.png", 600);
    ui.set_toast_shown(false);
    ui.invoke_te_open(8);
    snap(&win, &out, "03b-urine-editor.png", 700);
    ui.invoke_te_open(1);
    snap(&win, &out, "03e-cbc-editor.png", 500);
    ui.invoke_te_back();
    ui.set_s_tab(1); advance(300);
    ui.invoke_pick_design(4); advance(300);
    snap(&win, &out, "03j-report-designs.png", 500);
    ui.set_s_tab(2);

    // People tab: add a front-desk person (needs the Admin's password again)
    ui.set_s_tab(3);
    ui.invoke_ue_new();
    ui.set_ue_name("Fozia Shah".into()); ui.set_ue_user("fozia".into()); ui.set_ue_pass("123".into());
    ui.invoke_ue_save();
    snap(&win, &out, "05-people-error.png", 600);
    ui.set_ue_pass("desk1234".into());
    ui.invoke_ue_save();
    snap(&win, &out, "05b-confirm-add.png", 500);
    ui.set_confirm_pass("wrong".into());
    ui.invoke_confirm_yes_clicked();
    snap(&win, &out, "05c-confirm-wrong-password.png", 300);
    ui.set_confirm_pass("secret1".into());
    ui.invoke_confirm_yes_clicked();
    advance(600);
    println!("after add, dialog shown: {}, toast: {}", ui.get_confirm_shown(), ui.get_toast_text());
    // save a change without a new password: plain yes/no
    ui.set_ue_name("Fozia S. Shah".into());
    ui.set_ue_pass("".into());
    ui.invoke_ue_save();
    snap(&win, &out, "05d-confirm-save.png", 500);
    ui.invoke_confirm_no_clicked();
    ui.invoke_ue_save(); ui.invoke_confirm_yes_clicked();
    // the only Admin cannot demote themselves
    ui.invoke_ue_open(1);
    snap(&win, &out, "05e-sole-admin-locked.png", 600);
    // a throw-away person can be removed
    ui.invoke_ue_new(); ui.set_ue_name("Temp Person".into()); ui.set_ue_user("temp1".into()); ui.set_ue_pass("temp1234".into());
    ui.invoke_ue_save(); ui.set_confirm_pass("secret1".into()); ui.invoke_confirm_yes_clicked();
    ui.invoke_ue_remove();
    snap(&win, &out, "05f-confirm-remove.png", 500);
    ui.set_confirm_pass("secret1".into()); ui.invoke_confirm_yes_clicked();
    advance(500);
    println!("remove toast: {}", ui.get_toast_text());
    snap(&win, &out, "06-people.png", 600);

    // Sign out, wrong password, then the new person signs in
    ui.invoke_sign_out();
    snap(&win, &out, "07-login.png", 900);
    ui.set_l_user("fozia".into()); ui.set_l_pass("nope".into());
    ui.invoke_sign_in();
    snap(&win, &out, "08-login-error.png", 300);
    ui.set_l_pass("desk1234".into());
    ui.invoke_sign_in();
    println!("signed in: {} as {}", ui.get_signed_in(), ui.get_user_role());
    ui.invoke_navigate(3);
    snap(&win, &out, "09-desk-settings.png", 900);
    ui.invoke_sign_out();
    // forgot the Admin password: username + recovery code + new password
    snap(&win, &out, "09b-login.png", 600);
    ui.set_l_user("mutaib".into());
    ui.invoke_forgot_open();
    ui.set_f_code("AAAAA-BBBBB-CCCCC-DDDDD".into()); ui.set_f_pass("reset999".into()); ui.set_f_pass2("reset999".into());
    ui.invoke_recover();
    snap(&win, &out, "09c-forgot-wrong-code.png", 500);
    ui.set_f_code(code.to_lowercase().replace('-', "").into());
    ui.invoke_recover();
    println!("recovered: signed in {} as {}; new code shown {}", ui.get_signed_in(), ui.get_user_name(), ui.get_rc_shown());
    snap(&win, &out, "09d-new-code.png", 600);
    ui.invoke_rc_done();
    ui.invoke_sign_out();
    ui.set_l_user("mutaib".into()); ui.set_l_pass("secret1".into());
    ui.invoke_sign_in();
    println!("old password still works: {}", ui.get_signed_in());
    ui.set_l_pass("reset999".into());
    ui.invoke_sign_in();
    ui.set_s_tab(0);

    // Settings + letterhead
    ui.invoke_navigate(3);
    ui.set_s_name("Al-Shifa Diagnostic Centre".into());
    ui.set_s_address("Rajbagh, Srinagar, Kashmir".into());
    ui.set_s_phone("0194-2000000".into());
    ui.set_s_signatory("Dr. Example Name".into());
    ui.set_s_qualification("MD Pathology".into());
    ui.invoke_save_settings();
    if let Some(l) = &letterhead { ctl.set_image("letterhead", std::path::Path::new(l)); }
    snap(&win, &out, "10-settings.png", 1500);

    // Registering with the form empty: errors must appear beside the fields.
    ui.invoke_navigate(1);
    ui.invoke_register();
    snap(&win, &out, "11-form-errors.png", 600);
    ui.set_err_first("".into()); ui.set_err_age("".into()); ui.set_err_address("".into()); ui.set_err_tests("".into());

    // Register properly
    ui.set_f_first("Ali".into()); ui.set_f_last("Khan".into()); ui.set_f_age("42".into());
    ui.set_f_phone("9906000000".into()); ui.set_f_address("Rajbagh, Srinagar".into());
    ui.set_f_discount("50".into()); ui.set_f_paid("500".into());
    for id in [1, 2, 3] { ui.invoke_toggle_test(id); }
    ui.invoke_bill_changed();
    snap(&win, &out, "12-register.png", 800);
    ui.invoke_register();
    advance(1000);
    println!("page after register: {}", ui.get_page());

    // Results typed live
    let params = ui.get_params();
    let find = |name: &str| (0..params.row_count()).find(|i| params.row_data(*i).unwrap().name == name).map(|i| i as i32);
    for (name, v) in [("Haemoglobin", "12.1"), ("RBC count", "4.2"), ("Haematocrit (PCV)", "36"), ("Total WBC count", "12400"), ("Platelet count", "14000"), ("Glucose (fasting)", "112"), ("Total cholesterol", "214"), ("HDL cholesterol", "38"), ("Triglycerides", "180")] {
        if let Some(i) = find(name) { ui.invoke_value_edited(i, v.into()); } else { println!("row not found: {name}"); }
    }
    snap(&win, &out, "13-results.png", 800);
    ui.invoke_save_draft();
    ui.invoke_approve();
    advance(500);
    println!("approve summary shown: {}", ui.get_ap_shown());
    snap(&win, &out, "44-approve-summary.png", 500);
    ui.invoke_ap_confirm();
    snap(&win, &out, "14-approved.png", 800);
    ui.invoke_make_report();
    advance(600);
    println!("toast: {} (bad={})", ui.get_toast_text(), ui.get_toast_bad());
    snap(&win, &out, "15-report-toast.png", 500);

    ui.invoke_navigate(2);
    snap(&win, &out, "16-reports.png", 900);
    ui.set_s_tab(9); ui.invoke_navigate(3);
    snap(&win, &out, "16a-email-settings.png", 700);
    ui.invoke_navigate(9);
    snap(&win, &out, "16b-analytics.png", 900);
    ui.invoke_navigate(0);
    snap(&win, &out, "17-home.png", 1500);

    // Urine report: pick the Colour from the list, save, approve
    ui.invoke_navigate(1);
    ui.set_f_first("Sara".into()); ui.set_f_age("29".into()); ui.set_f_gender(1); ui.set_f_address("Baramulla".into());
    ui.invoke_toggle_test(8);
    ui.invoke_register();
    advance(800);
    let params = ui.get_params();
    let colour = (0..params.row_count()).find(|i| params.row_data(*i).unwrap().name == "Colour").unwrap() as i32;
    println!("Colour row is a dropdown: {}", params.row_data(colour as usize).unwrap().choice);
    ui.invoke_prepare_choices(colour);
    println!("choices offered: {}", ui.get_choices().row_count());
    ui.invoke_value_edited(colour, "Pale yellow".into());
    ui.invoke_save_draft();
    println!("after saving the colour: {} (bad={})", ui.get_toast_text(), ui.get_toast_bad());
    snap(&win, &out, "18-urine-sheet.png", 700);

    // The lab's standard list of tests
    ui.set_s_tab(2); ui.invoke_navigate(3);
    ui.invoke_load_samples();
    println!("tests offered now: {}", ui.get_tests_admin().row_count());
    snap(&win, &out, "19-standard-tests.png", 700);
    ui.set_s_tab(5);
    snap(&win, &out, "20-my-account.png", 600);

    // ---- new in this version ----
    // Registration: referred by a doctor + optional email, with the doctor's name remembered
    ui.invoke_navigate(1);
    ui.set_f_first("Nasir".into()); ui.set_f_last("Lone".into()); ui.set_f_age("51".into()); ui.set_f_address("Anantnag".into());
    ui.set_f_email("nasir@".into()); ui.set_f_ref(1);
    ui.invoke_toggle_test(1);
    ui.invoke_register();
    snap(&win, &out, "21-register-errors.png", 700);
    ui.set_f_email("nasir@example.com".into()); ui.set_f_doctor("Dr. Rashid Khan".into()); ui.set_f_paid("100".into());
    ui.invoke_register();
    advance(900);
    println!("registered with doctor, page {}", ui.get_page());
    snap(&win, &out, "22-sheet-buttons.png", 700);
    ui.invoke_make_bill();
    advance(400);
    println!("bill toast: {}", ui.get_toast_text());
    // another patient: the doctor now appears as a suggestion
    ui.invoke_navigate(1);
    ui.set_f_ref(1);
    ui.invoke_doctor_edited("rash".into());
    ui.set_f_doctor("rash".into());
    snap(&win, &out, "23-doctor-suggestion.png", 600);
    ui.set_f_ref(0); ui.set_f_doctor("".into());
    // correct the patient's details
    ui.invoke_open_order(ui.get_orders().row_data(0).map_or(1, |r| r.id));
    advance(500);
    ui.invoke_edit_patient();
    ui.set_e_phone("9906123456".into()); ui.set_e_email("bad email".into());
    ui.invoke_edit_save();
    snap(&win, &out, "24-edit-patient-error.png", 700);
    ui.set_e_email("nasir.lone@example.com".into());
    ui.invoke_edit_save();
    advance(600);
    println!("edit toast: {}", ui.get_toast_text());
    // delete it, then bring it back
    ui.invoke_delete_report();
    snap(&win, &out, "25-delete-confirm.png", 600);
    ui.invoke_confirm_yes_clicked();
    println!("delete needs reason: {}", ui.get_err_confirm_extra());
    ui.set_confirm_extra("entered twice by mistake".into()); ui.set_confirm_pass("reset999".into());
    ui.invoke_confirm_yes_clicked();
    advance(600);
    println!("delete toast: {} page {}", ui.get_toast_text(), ui.get_page());
    ui.set_s_tab(4); ui.invoke_navigate(3);
    snap(&win, &out, "26-data-backup.png", 900);
    println!("deleted rows: {}", ui.get_deleted().row_count());
    ui.invoke_restore_deleted(ui.get_deleted().row_data(0).map_or(1, |r| r.id));
    println!("restore toast: {}", ui.get_toast_text());
    // help + about
    ui.set_s_tab(6);
    snap(&win, &out, "27-help.png", 800);
    ui.set_help_query("delete".into());
    ui.invoke_help_edited("delete".into());
    println!("help hits for 'delete': {}", ui.get_help_items().row_count());
    snap(&win, &out, "28-help-search.png", 500);
    ui.set_s_tab(7); ui.invoke_navigate(3);
    snap(&win, &out, "29-about.png", 900);
    // returning patient: search, fill the form, register a second visit under the same ID
    ui.invoke_navigate(1);
    ui.set_f_find("khan".into()); ui.invoke_find_edited("khan".into());
    snap(&win, &out, "32-find-returning.png", 600);
    let first_hit = ui.get_find_results().row_data(0).map(|r| r.id).unwrap_or(0);
    ui.invoke_pick_patient(first_hit);
    snap(&win, &out, "33-returning-filled.png", 600);
    ui.set_f_age("43".into());
    ui.invoke_toggle_test(1);
    ui.invoke_register();
    advance(800);
    println!("second visit registered, page {}, id kept: {}", ui.get_page(), ui.get_r_sub());
    // second visit: type a different haemoglobin and approve, so the patient has a trend
    {
        let params = ui.get_params();
        if let Some(i) = (0..params.row_count()).find(|i| params.row_data(*i).unwrap().name == "Haemoglobin") { ui.invoke_value_edited(i as i32, "13.4".into()); }
        ui.invoke_save_draft(); ui.invoke_approve(); advance(400); ui.invoke_ap_confirm(); advance(400);
    }
    // the patient's whole history
    ui.invoke_navigate(6);
    snap(&win, &out, "34-patients.png", 700);
    ui.invoke_pick_pat(ui.get_pat_list().row_data(0).map(|r| r.id).unwrap_or(0));
    snap(&win, &out, "35-patient-history.png", 900);
    advance(400); snap(&win, &out, "35b-trend.png", 900);
    println!("trend names: {} picked '{}' points {} range '{}'", ui.get_tr_names().row_count(), ui.get_tr_pick(), ui.get_tr_points().row_count(), ui.get_tr_range());
    println!("history: {} visits, {} tree rows", ui.get_pat_visits(), ui.get_pat_tree().row_count());
    // typing a person who is already registered: hint while typing, and a question when registering
    ui.invoke_navigate(1);
    ui.set_f_first("Ali".into()); ui.set_f_phone("9906000000".into());
    snap(&win, &out, "36-same-person-hint.png", 600);
    println!("same-person hints while typing: {}", ui.get_match_results().row_count());
    ui.set_f_age("44".into()); ui.set_f_address("Srinagar".into());
    ui.invoke_toggle_test(1);
    ui.set_f_paid("100".into());
    ui.invoke_register();
    snap(&win, &out, "37-same-person-ask.png", 500);
    ui.invoke_confirm_no_clicked();      // "no, a new person"
    advance(800);
    println!("registered as a new person, page {}", ui.get_page());
    // pay the rest later
    snap(&win, &out, "38-sheet-owing.png", 500);
    ui.invoke_pay_open();
    snap(&win, &out, "39-pay-dialog.png", 500);
    ui.set_pay_amount("5000".into()); ui.invoke_pay_save();
    println!("too much: {}", ui.get_err_pay());
    ui.set_pay_amount("200".into()); ui.set_pay_method(1);
    ui.invoke_pay_save();
    advance(600);
    println!("pay toast: {} | {}", ui.get_toast_text(), ui.get_r_money());
    ui.invoke_navigate(6);
    ui.invoke_pick_pat(ui.get_pat_list().row_data(0).map(|r| r.id).unwrap_or(0));
    println!("patient can pay: {}", ui.get_pat_can_pay());
    if ui.get_pat_can_pay() {
        ui.invoke_pay_open_patient();
        snap(&win, &out, "40-pay-patient.png", 500);
        ui.invoke_pay_cancel();
    }
    // scanning a report's QR: the lab finds the patient and says whether the paper is genuine
    let qr = ctl.qr_text_for(1).unwrap_or_default();
    ui.invoke_navigate(2);
    ui.set_order_query(qr.as_str().into());
    ui.invoke_refresh_orders();
    advance(600);
    println!("scan: page {} verdict {} | {}", ui.get_page(), ui.get_verdict_kind(), ui.get_verdict_text());
    snap(&win, &out, "41-scan-genuine.png", 900);
    let forged = qr.replace("Ref: ", "Ref: 00");
    ui.invoke_navigate(2);
    ui.set_order_query(forged.as_str().into());
    ui.invoke_refresh_orders();
    advance(600);
    println!("forged scan verdict {}", ui.get_verdict_kind());
    snap(&win, &out, "42-scan-forged.png", 900);
    // payments list + combined bill, and the Scan screen
    ui.invoke_navigate(6);
    ui.invoke_pick_pat(ui.get_pat_list().row_data(0).map(|r| r.id).unwrap_or(0));
    snap(&win, &out, "45-patient-payments.png", 600);
    ui.invoke_print_full_bill();
    println!("full bill toast: {}", ui.get_toast_text());
    ui.invoke_navigate(7);
    snap(&win, &out, "46-scan-waiting.png", 600);
    ui.set_scan_input(qr.as_str().into());
    ui.invoke_scan_edited(qr.as_str().into());
    advance(600);
    println!("scan screen kind {} | {}", ui.get_sc_kind(), ui.get_sc_text());
    snap(&win, &out, "47-scan-result.png", 700);
    ui.set_scan_input(forged.as_str().into()); ui.invoke_scan_submit();
    println!("scan forged kind {}", ui.get_sc_kind());
    snap(&win, &out, "48-scan-forged.png", 700);
    ui.set_scan_input("hello".into()); ui.invoke_scan_submit();
    println!("scan foreign kind {}", ui.get_sc_kind());
    // analyzer results: a real analyzer message arrives, is looked at, and is attached to a report
    let sample = include_str!("../../labdesk-core/tests/fixtures/urit-3part-sample.hl7.txt");
    println!("analyzer message accepted: {}", ctl.inject_machine_message(sample));
    ui.invoke_navigate(8);
    advance(600);
    println!("analyzer list rows: {} new badge {}", ui.get_mach_list().row_count(), ui.get_mach_new());
    ui.invoke_mach_pick(ui.get_mach_list().row_data(0).map_or(0, |r| r.id));
    advance(600);
    println!("analyzer detail: {} | items {} graphs {} candidates {}", ui.get_md_name(), ui.get_md_items().row_count(), ui.get_md_hists().row_count(), ui.get_md_cands().row_count());
    snap(&win, &out, "50-analyzer-result.png", 900);
    if let Some(c) = ui.get_md_cands().row_data(0) { ui.invoke_mach_attach(c.order_test_id); advance(600); println!("attach toast: {}", ui.get_toast_text()); }
    snap(&win, &out, "51-analyzer-attached.png", 700);
    // second sample: one step from the analyzer result to a report, then approve and print
    let second = sample.replace("000000000096", "000000000097").replace("Mrs Sample Patient", "Mr Quick Example").replace("|40|F", "|55|M");
    println!("second message accepted: {}", ctl.inject_machine_message(&second));
    ui.invoke_navigate(8); advance(400);
    ui.invoke_mach_pick(ui.get_mach_list().row_data(0).map_or(0, |r| r.id)); advance(600);
    println!("quick form: name '{}' age '{}' gender {}", ui.get_qp_name(), ui.get_qp_age(), ui.get_qp_gender());
    snap(&win, &out, "52-analyzer-quick.png", 900);
    ui.invoke_mach_quick(); advance(800);
    println!("quick toast: {} | order {} can_approve {}", ui.get_toast_text(), ui.get_md_order(), ui.get_mach_can_approve());
    snap(&win, &out, "53-analyzer-ready.png", 700);
    ui.invoke_mach_print(); advance(600);
    println!("approve dialog shown: {}", ui.get_ap_shown());
    snap(&win, &out, "54-analyzer-approve.png", 700);
    ui.invoke_ap_confirm(); advance(1200);
    println!("after approve+print: {}", ui.get_toast_text());
    ui.set_s_tab(8); ui.invoke_navigate(3);
    snap(&win, &out, "43-machines.png", 700);
    // summary and export
    ui.invoke_navigate(2); advance(500);
    ui.invoke_sm_open(); advance(700);
    println!("summary rows: {} from {} to {} err '{}'", ui.get_sm_rows().row_count(), ui.get_sm_from(), ui.get_sm_to(), ui.get_err_sm());
    snap(&win, &out, "55-summary.png", 800);
    ui.invoke_sm_export("bills".into()); advance(300);
    println!("export toast: {}", ui.get_toast_text());
    ui.invoke_sm_close(); advance(300);
    // About page with the update button, and the crash window
    ui.set_upd_status("Version 0.2.0 is downloaded and checked. It installs the next time LabDesk starts, or press INSTALL NOW (finish anything you are typing first).".into());
    ui.set_upd_ready(true);
    ui.set_s_tab(7); ui.invoke_navigate(3); advance(500);
    snap(&win, &out, "56-about-updates.png", 700);
    ui.invoke_navigate(2); advance(300);
    ui.set_crash_text("LabDesk problem report\n======================\nVersion: 0.1.0\nSystem: windows x86_64\nData file size: 412 KB\n\nRecent actions\n--------------\n2026-11-06  Admin  approve order 12\n\nCrash notes\n-----------\n[2026-11-06T04:11:09Z] LabDesk 0.1.0 crashed at src/app.rs:1234: index out of bounds".into());
    ui.set_crash_shown(true); advance(500);
    snap(&win, &out, "57-crash-report.png", 700);
    ui.invoke_crash_close(); advance(300);
    // five wrong passwords lock sign-in
    ui.invoke_sign_out();
    ui.set_l_user("mutaib".into());
    for i in 1..=5 { ui.set_l_pass("wrong".into()); ui.invoke_sign_in(); if i == 4 { snap(&win, &out, "30-login-warning.png", 300); } }
    snap(&win, &out, "31-login-locked.png", 400);
    ui.set_l_pass("reset999".into());
    ui.invoke_sign_in();
    println!("signed in while locked: {} (error: {})", ui.get_signed_in(), ui.get_err_login());
}
