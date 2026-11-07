# Contributing to LabDesk

Thank you for wanting to help. LabDesk is used in real labs, so changes are checked carefully.

## Before anything else

- **Never post real patient data.** No names, phone numbers, reports, screenshots of real reports or database files in issues, pull requests or logs. Use made-up data.
- Security problems go to the private address in [SECURITY.md](SECURITY.md), not to a public issue.
- Be kind. See the [Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting a bug

Open an issue and choose **Bug report**. Say what you did, what you expected and what happened, plus the version from Settings > About. A problem report file saved from Settings > Help contains no patient data and is safe to attach.

## Suggesting a feature

Choose **Feature request** and describe the problem in the lab first, not only the solution. Small labs with one PC are the target, so features that need a server or an account are unlikely to fit.

## Sending code

1. Fork the repository and create a branch.
2. Install the stable Rust toolchain.
3. Make your change. Keep it small and focused.
4. Run the tests: `cargo test -p labdesk-core` and `cargo test -p labdesk-desktop --lib`. Add a test for any new rule.
5. Open a pull request and fill in the template.

### Rules the code follows

- Money is integer **paise**. Never use floating point for money.
- Days and hours follow **Indian Standard Time**.
- Every function that changes data takes the actor and calls `allow(actor, &[roles])` first. Add a test that a wrong role is refused.
- Changes to approved or delivered reports must go through the existing correction path and write to the audit log.
- Text shown to people is plain English that a front-desk person understands, with no technical words.
- Do not add network calls that run without a person asking for them.
- Do not add a dependency without saying why. Its licence must be compatible with GPL-3.0-only.
- Add this header to new source files:

```
// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib
```

### Licence of contributions

By sending a pull request you agree that your contribution is licensed under the **GNU GPL version 3 only**, the same as the project, and that you have the right to submit it.

## The website and manual

The website and manual are built from a separate project. Wrong or missing documentation can be reported as an issue here.
