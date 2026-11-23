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

The preview uses the same directory scan and Windows profile as `check`.
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
cannot open the directory produces no report. The JSON currently captures a
preview snapshot. Applying saved plans is a future feature.

The exporter uses [Serde](https://serde.rs/) and
[serde_json](https://docs.rs/serde_json/) for JSON mapping and escaping,
and [tempfile](https://docs.rs/tempfile/) to publish the completed file
without overwriting an existing output.

## Verify a saved preview

```powershell
cargo run --locked -- verify-plan plan.json
```

The command reads the saved plan and scans its directory again. It returns
exit code `0` if the saved proposals match the current ready preview, or `2`
if the plan cannot be verified. This includes unsupported versions, blocked
or incomplete scans, changed proposals, and newly occupied targets. Proposals
are compared as data. Saved source and target strings are never used as
filesystem paths during verification.

Verification checks the naming preview at the time of the scan. File identity,
changes to content at the same name, and filesystem changes after the scan
are outside its scope. The command performs no renames.
