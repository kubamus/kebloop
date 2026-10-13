# Performance profiling

Run `./scripts/profile-app.sh` on macOS with the selected Xcode toolchain. It builds in release mode and prints a `KEBLOOP_PROFILE` JSON record, then runs an isolated file-path benchmark with `/usr/bin/time -l` resource counters. The profile briefly opens the launcher; keep the desktop free of interaction until it finishes.

The AppKit profile uses a separate UserDefaults suite, a named synthetic pasteboard, and an empty clipboard history. It reads the real application index and its cache, but does not install apps, register global shortcuts, open results, send AI prompts, or modify login settings. It exercises 180 queries and 400 selection moves with 117 applications on the measured machine. Query timing includes synchronous ranking, row creation and layout; navigation timing includes selection and scrolling. It yields between batches so pending UI updates contribute to total workload CPU time. File searches in this typing workload are generally cancelled before Spotlight's debounce expires.

`peak_rss_mib` is the test process's peak resident memory, including XCTest/AppKit overhead. `workload_cpu_seconds` includes user and system CPU for the process, measured separately from elapsed wall time. `idle_cpu_percent` samples five seconds after hiding the panel, with clipboard recording disabled. Startup measurements use an existing on-disk app cache and are not cold system startup numbers. None of these numbers include Spotlight's separate process or system compositor cost.

For live sessions, the app emits Points of Interest intervals under subsystem `com.kebloop.launcher`: `Query results`, `Render rows`, and `File search`. Record the release app with Instruments' Time Profiler, Allocations, and Points of Interest instruments to relate slow typing or file searches to CPU and allocation activity. File search intervals cover debounce through completion; cancellation also ends the interval. Signposts contain no query text, file paths, or clipboard content.

The standalone file visibility benchmark checks 512 canonical paths 200 times. It isolates exclusion classification; its memory numbers are not the launcher's memory usage. `KEBLOOP_PROFILE=1 swift test -c release --filter SearchTests/testThousandAppSearchPerformance` separately times ranking 1,000 synthetic entries. Timing profiles are opt-in and have no machine-dependent pass/fail latency threshold.

Optimizations in this change:

- Keyboard navigation updates two existing row highlights instead of recreating all rows and constraints.
- Hidden panels skip reactive ranking/rendering; unchanged file-search publications and irrelevant status renders are suppressed.
- Search normalizes names once per candidate and avoids temporary fuzzy-score arrays.
- Opening the launcher uses a five-minute app-index freshness window; explicit refresh still scans immediately.
- Icons have a count cache limit and only the first 32 applications are warmed. Other icons load on demand.
- File exclusions reuse constants and avoid repeated Foundation path conversions for canonical paths.
- Clipboard polling stops while disabled. The serial persistence writer retains only one active and one latest pending snapshot, avoiding a backlog of full-history saves. A controlled 500-update burst test produces two writes rather than 501; clear/remove remain the final saved state.

## Local results

Measured on arm64 macOS 27.0, Swift 6.4, release builds on 2026-10-12. Baseline source commit: `8bdf210da6aec3f36eaf8fd84c5a2a6833571b1f`. Baseline was built in an isolated temporary checkout with the same profile entry points. The raw samples are in [performance/local-results.json](performance/local-results.json).

Median of three runs per version (p95 rows are the median of each run’s p95):

| Metric | Baseline | Optimized |
| --- | ---: | ---: |
| Query median | 5.165 ms | 3.361 ms |
| Query p95 | 11.257 ms | 10.415 ms |
| Navigation median | 5.220 ms | 0.061 ms |
| Navigation p95 | 8.549 ms | 0.081 ms |
| Workload CPU | 5.529 s | 1.049 s |
| Workload elapsed | 5.988 s | 1.212 s |
| Peak process RSS | 161.203 MiB | 121.406 MiB |
| First panel show | 73.331 ms | 68.357 ms |
| Index ready | 413.676 ms | 374.689 ms |
| Hidden idle CPU | 0.114 % | 0.249 % |

The measured workload used 81% less CPU time and 25% less peak resident memory. Navigation improved by 86.1×. Query p95 changed only modestly. Idle CPU varied between runs and did not improve; no idle-CPU reduction is claimed. Startup is sensitive to disk cache and desktop load. These local measurements are not guarantees for other Macs or large clipboard/quicklink collections.

The standalone visibility benchmark’s repeated baseline runs took 2,111–2,158 ms versus 639–647 ms after optimization for 102,400 checks (about 3.3× faster). The final profiling script measured 625 ms. Its process RSS was approximately 69.2 MB before and 7.3 MB after; this is isolated allocation pressure, not a measurement of the app.

The initial 48 MiB icon cost budget incorrectly counted all native image representations and could immediately evict a single icon. It was removed after the missing-icon regression was reproduced. The cache keeps its 128-image count limit and 32-image warmup; the measurements above describe the earlier profiling revision and should be re-run for the icon fix.

A [single follow-up run with icons restored](performance/icon-fix-results.json) measured 126.25 MiB peak process RSS, 1.245 s workload CPU, and 0.077 ms median navigation. Query median was 5.570 ms and p95 12.258 ms; the original query-latency improvement should not be assumed with icons restored.

## Milestones 1–2 follow-up

A single release profile after the personalization/workflow implementation on the same arm64 macOS 27 machine measured query median **5.445 ms**, p95 **11.925 ms**, navigation median **0.081 ms**, workload CPU **1.426 s**, and peak RSS **130.41 MiB**. The standalone visibility benchmark took **635.63 ms**. Raw results are in [milestones-1-2-results.json](performance/milestones-1-2-results.json).

Compared with the single icons-restored sample, query and navigation latency remain similar; workload CPU increased from 1.245 s to 1.426 s and RSS from 126.25 to 130.41 MiB. The empty launcher now includes more built-in commands, so the workload renders more rows. First show measured 120.16 ms and index readiness 429.62 ms; startup remains sensitive to desktop and cache state. This is one local sample, not a cross-machine performance guarantee. The profile supplies isolated personalization and Apple Shortcuts stores and does not run user automations.

A follow-up after the custom action panel and Settings redesign measured query median **6.217 ms**, p95 **13.570 ms**, navigation median **0.080 ms**, workload CPU **1.328 s**, and peak RSS **130.27 MiB**. The file visibility benchmark took **612.83 ms**. This single sample has somewhat higher query latency than the milestone sample, with lower workload CPU and similar memory/navigation; no systematic speedup is claimed. Raw results are in [actions-settings-results.json](performance/actions-settings-results.json).

## Milestones 3–4 follow-up

The release AppKit profile on the same arm64 macOS 27 host measured query median **4.699 ms**, p95 **12.410 ms**, navigation median **0.081 ms**, workload CPU **1.444 s**, and peak RSS **132.14 MiB**. This single sample is comparable with the prior actions/settings sample (6.217 ms median, 13.570 ms p95, 130.27 MiB RSS); no systematic speedup is claimed. The synthetic profile uses isolated empty snippet/script/extension stores and does not execute local automation. Raw data: [milestones-3-4-results.json](performance/milestones-3-4-results.json). Attachment thumbnails are decoded off the UI thread, coalesced by attachment/size, and kept in a bounded cache that is invalidated when history changes.

A subsequent feature-verification profile measured query median **4.825 ms**, p95 **12.716 ms**, navigation median **0.081 ms**, workload CPU **1.387 s**, and peak RSS **131.98 MiB**. The profile passed and remains a single local sample; raw data is in [feature-verification-results.json](performance/feature-verification-results.json).

## Larger workflow follow-up

On 2026-10-13 the new workflow registry exposed an empty-panel layout cost: rendering every command produced query p95 **30.98 ms** and **29.97 ms** in two release profiles, with roughly **155.5 MiB** peak test-process RSS. The empty launcher now offers eight everyday commands alongside the user's configured apps and favorites; all commands remain searchable and available in Browse Commands.

A profile of the final implementation measured query median **4.625 ms**, p95 **14.232 ms**, navigation median **0.078 ms**, workload CPU **1.460 s**, and peak RSS **131.30 MiB**. The standalone file visibility benchmark measured **648.84 ms**. This is close to the recorded feature-verification sample (4.825 ms median, 12.716 ms p95, 131.98 MiB RSS); no systematic speedup is claimed. Utility services and the Raycast runtime start only when invoked and are not executed by the typing profile. [Raw samples](performance/workflow-results.json) retain the two pre-adjustment measurements and the final result.

## Keyboard-first utilities follow-up

The launcher now includes Dictionary and Dictation among ten bounded default command suggestions, and hosts their views inline. The 2026-10-13 release profile measured query median **5.119 ms**, p95 **15.276 ms**, navigation median **0.075 ms**, workload CPU **1.506 s**, and peak test-process RSS **132.86 MiB**. Compared with the preceding workflow sample (4.625 ms median, 14.232 ms p95, 131.30 MiB RSS), the values remain close; this single local sample is not a cross-machine guarantee. Dictionary lookup runs off the UI thread; dictation and notes are activated only on command. [Raw result](performance/keyboard-utilities-results.json).
