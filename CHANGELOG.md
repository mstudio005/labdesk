# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/) (while the number starts with 0, any release may change things).

## [Unreleased]

## [1.0.0] - First release

The first public version of LabDesk.

### Added
- **Terms of Use and Privacy** (`TERMS.txt`): shown and accepted in the installer and the first time the program starts, and again whenever the text changes.
- Analytics page (Admin and Front desk): visits, money, busy hours, payment methods, departments, tests and referring doctors, with date ranges and comparison with yesterday.
- Email sending (Settings > Email, off by default) with Proton Mail, Gmail, Outlook and custom SMTP presets.
- LabDesk Bridge browser extension and the local listener it talks to (127.0.0.1, one-time code per report), so the WhatsApp button can attach the PDF for you.
- Bills: the lab's name, address or letterhead is no longer printed on a bill unless you switch **BILL HEADER** on (off by default, remembered). **BILL** chooses normal A4 paper or a thermal roll (80 mm or 58 mm), also remembered.
- Open-source release files: licence, contributing guide, security policy, issue and pull request templates, third-party notices.

### Changed
- The analyzer range choice is now named "Same as the analyzer" and stored as `analyzer`. Older databases that stored another value keep working and use the analyzer's own ranges.
- Source files carry SPDX licence headers.

## [0.1.7]

### Added
- Analytics core (queries and totals) with tests.

## [0.1.0] to [0.1.6] (private development releases)

### Added
- Patients, returning patients, bills, payments, discounts and balances.
- Result entry with normal ranges by age and gender, calculated lines, HIGH / LOW / CRIT marks.
- Approval, correction with a reason, delivery lock, reversible deletes.
- Reports with letterhead, eight patient-box designs, QR code and Ref code; QR scan verification.
- Roles, sign-in lock after wrong passwords, recovery code, audit log.
- Daily backups, cloud-folder copy, password-locked backups, restore.
- Blood cell counter link (HL7) with graphs.
- Automatic updates with a checksum check, crash report with one-click problem file.
- Summary and CSV export.
