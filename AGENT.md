When adding new features, always write tests first and watch the tests fail. This is important. Then fix the code so the tests pass. Unless it's impossible to test, of course.

Also, every added feature should be scriptable.

Use the app's scriptability to test issues. As you do this and find deficiencies in the scripting feature's abilities, file issues to fix those problems (and fix them if you can).

Run `cargo test` to make sure tests pass. Fix warnings from `cargo build` as they come up. Also sometimes run `cargo run --exit` to make sure it launches.

We're in pre-alpha right now, so don't worry about defining schema migrations at this point -- just alter the initial schema. And don't mind backward compatibility. Feel free to make breaking compatibility changes.

For every completed task/feature/fix, record the change with `changer add ...` (see `changer add --help` for more info)

When adding or changing a keyboard shortcut, update `shortcuts::all_shortcuts()` in the same change — it renders the Keyboard Shortcuts window (View/Help menus) and is the single source for the app's binding list (see SPEC §11).

When writing docs, I really *really* **REALLY** prefer brevity. Be as succinct as possible in the docs, but also thorough. Follow the "Writing style" principles in `docs-site/README.md` — the app is the documentation; docs only cover what the app can't show. Don't reference todoer issue numbers in the docs, but please reference them in the Git Commits.

Always work directly on the `master` branch. Do not create feature branches unless I specifically tell you to — commit your work straight to `master` and push to `origin` when a task is complete.

Never, ever install software on a computer. You can ask me to, but you do not do that.
