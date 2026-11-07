## What does this change, and why?

<!-- One or two sentences, in plain words. -->

## How was it tested?

- [ ] `cargo test -p labdesk-core`
- [ ] `cargo test -p labdesk-desktop --lib`
- [ ] I tried it in the program (if it changes a screen)

## Checklist

- [ ] No real patient data anywhere in the change (code, tests, screenshots, logs)
- [ ] Money stays in integer paise; times follow IST
- [ ] Anything that changes data checks the person's role first and has a test for a refused role
- [ ] Text shown to people is plain English
- [ ] New source files have the SPDX header
- [ ] I agree my contribution is licensed under GPL-3.0-only
