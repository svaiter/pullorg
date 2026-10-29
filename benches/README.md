# Benchmarks

[Criterion](https://bheisler.github.io/criterion.rs/book/) benchmarks for the
parser and the HTML renderer, plus a deterministic allocation report. All
inputs are generated (`benches/common/mod.rs`) from a fixed seed, so nothing
large is checked in and two machines measure the same bytes.

```sh
cargo bench                              # everything (20-30 minutes)
cargo bench --bench parse                # one target
cargo bench --bench parse -- parse/notes # one benchmark (the filter is a regex)
cargo bench -- --quick                   # fewer samples, a rough first look
cargo bench --bench alloc                # allocation report (seconds)
cargo test --benches                     # run every benchmark once, as CI does
```

Results land in `target/criterion/`; Criterion prints each estimate with a
confidence interval and, when a previous run exists, the change against it.
Throughput is in bytes of Org input per second everywhere, so inputs of
different sizes compare directly.

## Targets

| Target     | Group(s)                                      | Measures                                                                 |
|------------|-----------------------------------------------|--------------------------------------------------------------------------|
| `parse`    | `parse`, `parse-offsets`, `parse-collect`     | `Parser::new(..)` drained to the end; with byte ranges; collected to a `Vec` |
| `html`     | `html`, `html-render-only`, `html-write-io`   | parse + render as the CLI does; the renderer alone on collected events; the `io::Write` path |
| `elements` | `elements/<kind>`                             | 16 KiB of a single element kind: headlines, lists, tables, blocks, drawers, keywords, ... |
| `objects`  | `objects/<kind>`, `objects-options/<opt>`     | 8 KiB of a single object kind through `Parser::new_inline`; the options that change the object grammar |
| `scaling`  | `scaling/<doc>/<size>`, `scaling-adversarial/<case>/<size>` | the same input at 4, 16, 64 and 256 KiB; throughput must stay flat |
| `alloc`    | (plain binary, no Criterion)                  | heap allocations and borrowed-text ratio per corpus, see below |

## Corpora

Document corpora, 64 KiB each by default (`common::documents`):

- `notes`: a realistic notes file. An outline with TODO keywords, priorities,
  tags, planning lines and property drawers; sections of lightly marked-up
  paragraphs, lists, a table, a source block or a quote. **The number to
  look at first.**
- `prose`: plain words wrapped at 72 columns, no markup character anywhere.
  The floor: how fast text with nothing to find is scanned.
- `markup`: paragraphs where one word in four is an inline object of some
  kind. Object parser under load.
- `blocks`: element-heavy, few objects: nested lists, long tables, source
  and example blocks, drawers, dynamic blocks, footnote definitions, LaTeX
  environments, affiliated keywords.
- `outline`: headlines only, every headline feature, levels up to eight.
  Headline parser and tree flattening alone.
- `fixtures`: all of `tests/fixtures/*.org` concatenated and repeated. Small
  hand-written documents that together cover the whole syntax.

Adversarial corpora (`common::adversarial`) are built so that a careless
implementation goes quadratic or otherwise far slower per byte than on a
normal document: unclosed emphasis, link and LaTeX openers, one 64 KiB line,
a paragraph per two bytes, runs of blank lines, thirty-star headlines,
lists nested 24 deep, 200-column tables, unterminated blocks and drawers,
footnote references and definitions by the thousand, radio targets that
match common words, lines starting with `*` that are not headlines,
orphaned affiliated keywords, nested sub/superscripts, non-ASCII text.
`scaling-adversarial` runs each at four sizes.

To benchmark real documents, point `PULLORG_BENCH_CORPUS` at a directory:
every `.org` file in it is added to `parse/<name>`, `html/<name>` and the
allocation report. The Org manual (`doc/org-manual.org` in the Org
repository, GPL, so not checked in here) is a good large one.

```sh
mkdir -p /tmp/corpus && cp ~/org-mode/doc/*.org /tmp/corpus
PULLORG_BENCH_CORPUS=/tmp/corpus cargo bench --bench parse -- 'parse/org-'
```

## Comparing two commits

Criterion compares against the last run of the same benchmark by default,
which is what you want while iterating on one change. To compare branches,
save a named baseline:

```sh
git checkout main
cargo bench --bench parse -- --save-baseline main
git checkout my-branch
cargo bench --bench parse -- --baseline main
```

`tools/bench-compare.sh REF [cargo bench args...]` does this in one go: it
builds `REF` in a temporary git worktree, runs the benchmarks there as the
baseline, then runs them in the working tree against it. Nothing in your
checkout is touched.

```sh
sh tools/bench-compare.sh main --bench parse -- parse/notes
```

The machine matters more than the statistics: close heavy applications, plug
in a laptop, and treat changes under about 5 % as noise unless they repeat.
Criterion's `--noise-threshold` and `--significance-level` tune what it
reports as a change.

## Allocation report

`cargo bench --bench alloc` prints, per corpus, the heap allocations made by
`Parser::new(..).count()` (excluding the input), the bytes they requested,
allocations per KiB of input, the share of `Text`/`Code`/`Verbatim` events
that borrow from the input, and the same allocation counts for parse +
`push_html`. The numbers are exact and deterministic, so a diff of two
reports is a reliable regression check, which timings are not. `--json`
prints one object per line for scripting; a bare word filters by corpus
name.

The library forbids `unsafe`; the counting allocator in this one bench is
the only `unsafe impl` in the repository, a pass-through to `System` that
increments two atomics.

## Adding a benchmark

- A new element or object kind: add a line to `inputs()` in `elements.rs`
  or `objects.rs`. Keep the unit well formed, so that the input measures the
  recognised syntax, not a fallback path.
- A new pathological pattern: add a `Corpus` to `adversarial::all` in
  `common/mod.rs`; it is picked up by `scaling` and `alloc` automatically.
- A new document shape: add a generator to `common/mod.rs` and list it in
  `documents()`; it then runs in `parse`, `html`, `scaling` and `alloc`.

Every benchmark must run under `cargo test --benches` (CI does) and must be
deterministic: no randomness outside `common::Rng`, no clock, no files
outside `tests/fixtures` unless opted in through `PULLORG_BENCH_CORPUS`.
