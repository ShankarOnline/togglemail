# Contributing to ToggleMail

Thank you for helping. ToggleMail is a free family project by Toggle Minds, and contributions of
every size are welcome: bug reports, translations, provider settings, code and documentation.

## Ways to help

- **Report a bug or ask for a feature:** open an issue. For mail problems, say which provider and
  whether it is IMAP or POP3; never paste passwords or full message contents.
- **Add a mail provider:** provider settings live in `src-tauri/src/accounts.rs` (`providers()`).
- **Improve the window:** `ui/` is plain HTML, CSS and JavaScript with no build step.
- **Package it for another system:** see `.github/workflows/build.yml` and `snap/`.

## Making a change

1. Fork the repository and create a branch.
2. Build and run it: see "Building locally" in the [README](README.md).
3. Run the tests: `cd src-tauri && cargo test --lib`.
4. Open a pull request describing what changed and how you tested it.

Keep the style of the surrounding code, and keep changes focused.

## Licence of contributions

ToggleMail is licensed under the GNU GPL version 3 or later, with an additional permission for app
stores (see [COPYING.md](COPYING.md)). By opening a pull request you agree that your contribution is
licensed under those same terms, including the app-store permission, and that you have the right to
contribute it. Add a `Signed-off-by: Your Name <email>` line to your commits (`git commit -s`) to
confirm this ([Developer Certificate of Origin](https://developercertificate.org/)).
