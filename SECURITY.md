# Security policy

LabDesk keeps medical data, so security reports are taken seriously.

## Supported versions

Only the latest release is supported. Update from Settings > About, or download the newest installer.

## Report a vulnerability privately

Please **do not open a public issue** for a security problem. Write to **syedmutaibnazir@gmail.com** with:

- what you found and where,
- the steps to repeat it (use made-up data only),
- the LabDesk version.

You can expect a first reply within 7 days. We will tell you when it is fixed and, if you wish, thank you in the changelog.

## What is in scope

- Bypassing sign-in, roles or the lock after wrong passwords.
- Changing an approved or delivered report without leaving a record.
- Reading or changing data from another program or a web page through the local bridge (127.0.0.1, ports 47653 to 47662).
- Weak handling of backup passwords or the email password.
- Forged reports that still pass the QR verification.

## Good to know

- LabDesk stores everything in one local file. Anyone with full access to the computer and its Windows account can read that file. Use a Windows password and disk encryption.
- A locked backup cannot be opened without its password. That is by design.

## Please do not

- Test against a real lab's data.
- Share a vulnerability publicly before it is fixed.
