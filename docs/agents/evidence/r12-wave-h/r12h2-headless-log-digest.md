# r12h2 session digest - observation data, not delivery evidence

Source: .scratch/r12-wave-h-grill/evidence/r12h2-session/headless.log.2026-09-25 (497KB raw, gitignored).
Session: 2026-09-25 real dev GUI + live headless + resin; post-B-read-conn first read.
Result: ZERO db lock-wait >=1ms events across 348x256-row replace_ports + ~380 upserts + ~440 reads.

Filtered WARN/ERROR/DbPool lines (1):

```text
[2m2026-09-25T13:02:29.602649Z[0m [33m WARN[0m [2megressapikey_app::commands::backup[0m[2m:[0m config_import: whitebox written; reconcile failed [3merror[0m[2m=[0mactivate config: 系统找不到指定的文件。 (os error 2)
```
