# fntidy
A Rust CLI for Windows 11 that checks and fixes file and folder names for cross-platform and cloud compatibility, with previews, collision detection, and undo.

## Development

With a stable Rust toolchain installed, build and run the current CLI:

```sh
cargo build --locked
cargo run --locked -- --help
cargo run --locked -- --version
cargo test --locked
```

## Check a directory

```powershell
cargo run --locked -- check "D:\Export"
```

The read-only `check` command inspects direct file and folder names with the
`windows` profile. Findings identify the relative name and the rule. The
selected directory itself is outside the naming check. Links and other
reparse points are skipped and reported on stderr. They still occupy a sibling
name for collision detection.

| Rule ID | Condition |
| --- | --- |
| `WIN_INVALID_CHARACTER` | A Windows-forbidden punctuation character or a control character U+0000–U+001F |
| `WIN_TRAILING_DOT_OR_SPACE` | A final period or ASCII space |
| `WIN_RESERVED_DEVICE` | A reserved device name, including names with extensions and COM/LPT suffixes ¹, ² or ³ |
| `WIN_NAME_COLLISION` | Another sibling name matches after removing trailing periods and ASCII spaces and comparing without case |

The collision check targets typical case-insensitive Win32 naming. On Windows it
uses ordinal comparison. Other development platforms approximate it with simple
uppercase mappings. This check does not model volume-specific case rules or path
length limits. Names that cannot be represented as Unicode make the check
incomplete.

Exit codes are `0` for no violations, `1` for violations, and `2` for an invalid
invocation or incomplete check. Code `2` takes precedence when both apply.

Windows naming rules are based on
[Microsoft's file naming documentation](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file).
[Detox](https://github.com/dharple/detox) is a reference for filename cleanup.

## Preview renames

```powershell
cargo run --locked -- plan "D:\Export"
```

The preview uses the same directory scan and naming profile as `check`.
It replaces each forbidden character with an underscore, removes trailing
periods and ASCII spaces, and adds a leading underscore to reserved device
names and names that would otherwise become empty. For example, `CON.txt`
becomes `_CON.txt`. Existing Unicode and valid extensions are preserved.

Targets must fit within 255 UTF-16 code units. A target that matches an existing
sibling or another proposed target is blocked. Colliding names without a
default repair also block the preview. Exit code `0` means the preview
completed, `1` means a conflict or target needs manual review, and `2` means
the invocation was invalid or the scan was incomplete. The preview performs
no renames.

To save the preview to a new UTF-8 JSON file:

```powershell
cargo run --locked -- plan "D:\Export" --out plan.json
```

An existing output file is preserved and the command returns exit code `2`.
The JSON contains `schema_version`, `root` (the absolute directory path),
`profile`, `profile_version`, `status` (`ready`, `blocked`, or `incomplete`),
`proposals`, and `collisions`. Each proposal records the relative `source`
and `target` names, rule IDs, and a status of `proposed`, `invalid_target`,
`proposed_collision`, or `existing_sibling`. Collision groups list the
relative names of all involved entries. Blocked and incomplete scans can be
exported for inspection and retain their corresponding exit code. A scan that
cannot open the directory produces no report. The JSON captures a preview
snapshot for verification and batch apply on Windows.

The exporter uses [Serde](https://serde.rs/) and
[serde_json](https://docs.rs/serde_json/) for JSON mapping and escaping,
and [tempfile](https://docs.rs/tempfile/) to publish the completed file
without overwriting an existing output.

## Opt-in cleanup profile

Use `--profile tidy` with either command to apply extra naming rules in the
check or preview. The default profile is `windows`.

```powershell
fntidy check "D:\Export" --profile tidy
fntidy plan "D:\Export" --profile tidy --out tidy-plan.json
fntidy verify-plan tidy-plan.json
```

- `TIDY_EMOJI` removes recognised Unicode emoji sequences, including flags
  and joined emoji, treating them as word separators.
- `TIDY_WHITESPACE` trims Unicode whitespace, collapses internal runs to a
  single ASCII space, and removes spaces next to periods.
- The Windows repairs still apply. Empty names and newly exposed leading
  periods get an underscore where needed. Accents, other writing systems,
  ordinary punctuation and typical extensions are preserved.

For example, `März  🇨🇭.tar.gz` becomes `März.tar.gz`. If that target
already exists, the plan is blocked. A saved plan records its profile and
version, which verification uses to rebuild the preview. On Windows,
`apply` uses the same verification for file and directory renames.

## Portable POSIX filenames

Use `--profile posix` with `check` or `plan` to use the portable POSIX
filename character set: ASCII letters, digits, periods, underscores, and
hyphens. The profile also flags a leading hyphen because command-line tools
may interpret it as an option. Empty names, `.`, and `..` are invalid.

`plan` preserves portable characters and valid extensions. It transliterates
German umlauts (for example, `Ü` to `Ue`) and sharp S, removes accents from
letters with an ASCII base, and replaces each remaining run of unsupported
characters with an underscore. It prefixes a leading hyphen with an
underscore. For example, `Überblick 2026.pdf` becomes `Ueberblick_2026.pdf`.

A name with a violation and no proposed result containing a letter or digit
needs a manual rename. Such names and collisions block the plan. On Windows,
Windows naming rules also apply to proposed targets. Saved plans record
`posix` and are verified with the same profile.

## Check subdirectories

Use `--recursive` on `check` or `plan` to include all subdirectories.
It can be combined with any profile.

```powershell
fntidy check "D:\Export" --recursive --profile tidy
fntidy plan "D:\Export" --recursive --profile tidy --out "D:\tidy-plan.json"
```

The preview groups changes by folder and shows the original name directly
above its proposed replacement. Folder paths are relative to the selected
directory. The display marks repeated and edge spaces with `␠` and escapes
invisible control characters.
Collisions are checked among siblings. The scan skips symbolic links and
other reparse points. A directory that cannot be read makes the scan incomplete.
For a ready plan, an entry whose proposed path changes through a parent
rename also shows its final path. The preview counts proposed file and
folder renames and entries involved in a conflict. An existing entry
that occupies a target can contribute to the blocked count.

Recursive exports use schema version 3 and record the scan scope and the
identity of each proposed source's parent. `verify-plan` repeats the
recursive scan. `apply` uses that same scope on Windows.

## Verify a saved preview

```powershell
cargo run --locked -- verify-plan plan.json
```

The command checks the saved proposals against a fresh scan and compares
the filesystem identity of the root directory and each proposed source.
It returns exit code `0` on success or `2` if verification fails. Failures
include changed proposals, newly occupied targets, and replaced entries.
Identity checks use the fresh scan's source names. The command performs no
renames.

Ready non-recursive exports use schema version 2 and store the root identity and one
source identity per proposal in matching order. Regenerate earlier exports
before verifying them. Blocked and incomplete previews omit identities.
If an identity cannot be read, exporting a ready plan or verifying it fails.

On Windows, the identity includes volume serial number, 128-bit file ID
and creation time. Linux development uses device and inode numbers.
A matching identity is a point-in-time check: filesystems can reuse
identifiers, edits in place can preserve identity, and execution still
needs its own checks.

The Windows identity query uses [windows-sys](https://docs.rs/windows-sys/)
for Microsoft's [FileIdInfo API](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info).

## Windows preview

The Windows x64 GitHub Actions run packages the executable with
[German quick-start instructions](START-HERE.md).

On Windows, `fntidy apply plan.json` applies the file and directory renames
in a saved plan. It validates the entire plan before changing any names.
Deeper entries are renamed before their parent directories. Each step is
recorded in the journal under `%LOCALAPPDATA%\fntidy\runs` before the
rename, with a separate confirmation afterwards.
The result counts changed files and folders. If a run stops, the reported
counts include the steps confirmed in the journal.

Exit code `2` means the plan was rejected or the journal could not be
created. Exit code `3` means the run needs review. If a target becomes
occupied during the run, fntidy preserves the existing entry and stops.
Completed steps remain in effect. Use the journal path printed by fntidy
to review confirmed steps and the pending step before making further changes.

## Inspect or undo a run

Use the full path to `run.jsonl` printed by `apply`.

```powershell
fntidy recover "C:\Path\To\run.jsonl"
fntidy undo "C:\Path\To\run.jsonl"
```

`recover` reads the journal and reports the run's progress. If the last step
was interrupted, it compares the recorded entry identity with the entries
at both names and reports whether the step took effect.
Recovery compares the Windows volume and file ID because a rename can
change the creation timestamp on NTFS.

`undo` resolves a pending apply step and restores confirmed changes in
reverse order. Each undo step is written to the same journal before the
rename and confirmed afterwards. If undo is interrupted, run the same
command again to continue. If the original name is occupied or the entry
was moved or replaced, fntidy stops and preserves the existing entries.
Use `recover` to inspect the progress and check the reported paths before
retrying. Running `undo` again after completion reports that the run has
already been undone.

These commands return `0` on success, `2` if the journal cannot be opened
or parsed, and `3` if a filesystem conflict or write failure needs review.
Journals from the earlier preview remain readable. For those runs, fntidy
checks the recorded root and entry identities. New runs also record the
parent directory's identity for each step.
