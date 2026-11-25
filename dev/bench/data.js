window.BENCHMARK_DATA = {
  "lastUpdate": 1795606526004,
  "repoUrl": "https://github.com/nobuddyorg/PeekBuddy",
  "entries": {
    "benches (windows-2025)": [
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "959a6c291ee389ff5eae9e7c8d361ec9f92f5dea",
          "message": "test(store): kill writer mutants (#239)\n\nTests for the error texts, id sequence, page count, WAL\ntruncation, catalog rows after restart and rotation, size cap\nwith WAL, kind column, node file ids, carried cursors and\nthe system-row flush. Equivalent mutants are excluded with\nreasons.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-10-31T05:48:41Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/959a6c291ee389ff5eae9e7c8d361ec9f92f5dea"
        },
        "date": 1793435677815,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1874.5600568600612,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 692.5435987855188,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3457.2296890164153,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3560.469579911343,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5961.9268159091425,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3149321339779528,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.990241380390553,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3526694.53125,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 64545.936798318085,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4493045.238095239,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "825e1de0ab35a6359d6c9fc95ff43f9df9c90311",
          "message": "ci: fix heavy cases, Miri time and the vhd check (#243)\n\n* ci: lower the nightly heavy cases and give Miri time\n\nAt 200 cases three property tests time out on the hosted\nrunner; 40 fits. Miri was cancelled at 45 minutes.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test(engine): check the vhd is mounted before watching\n\nThe nightly detach said already detached and the watch never\nchanged state, so the test may have watched a plain folder.\nFail early with diskpart's own view when the mount folder is\nnot on the virtual disk.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-10-31T13:02:32Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/825e1de0ab35a6359d6c9fc95ff43f9df9c90311"
        },
        "date": 1793525196222,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1871.5696020061541,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 717.2021561282603,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3480.5063371829074,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3575.078238200682,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5986.097022282191,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3145999974794352,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.989855056536506,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3542867.619047619,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 64329.60114788594,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4562301.041666666,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "d10cfdc4c1f9c66139f6ca0f2d8650f0045ee4b3",
          "message": "test: record real traces, corpus and contracts (#287)\n\nCloses #270",
          "timestamp": "2026-11-02T09:37:50Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/d10cfdc4c1f9c66139f6ca0f2d8650f0045ee4b3"
        },
        "date": 1793612487612,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1960.820700046032,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 1212.9675731390937,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 2885.5222234447992,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 2952.258505670566,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5501.83061870189,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.35499447206402807,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 6.197647650748151,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 22.349576831538926,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3633703.854166667,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 62469.72986937831,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4457973.541666667,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4e36337144cb7c4bd3cec1cfab240129a21fdf4d",
          "message": "fix(engine): watch a lost root's remaining path (#312)\n\nPart of #277",
          "timestamp": "2026-11-03T09:23:44Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4e36337144cb7c4bd3cec1cfab240129a21fdf4d"
        },
        "date": 1793705364210,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1873.3405846213182,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 719.0210679563037,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3479.2637608800433,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3575.9754178140765,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5978.475634281143,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.31581916687465916,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.991214007217202,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 20.818739698024963,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3815522.7604166665,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 64364.11520431542,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4491316.555059524,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4e8f1ad0cffccf28d49a35b70acbc63c732d39b6",
          "message": "test(store): pin the log size and the pragmas (#334)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-04T10:59:55Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4e8f1ad0cffccf28d49a35b70acbc63c732d39b6"
        },
        "date": 1793793028296,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1742.7199986475434,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 1027.0438980819965,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3281.462721019409,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3108.951263848533,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5297.630463520141,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3304853755483037,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 13.823501997932109,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 24.039460267013332,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3673237.261904761,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 49541.58491988224,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3885970.8333333335,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1fb32b54db3a63e72f95019ed223b13dad455660",
          "message": "feat(viewer): fall back to wgpu without OpenGL (#349)\n\nx64 starts with glow and starts again with wgpu/DX12 when\nglow cannot start. CI runs the fallback on hosted runners.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-05T09:06:53Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/1fb32b54db3a63e72f95019ed223b13dad455660"
        },
        "date": 1793874932112,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1871.6906174830451,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 718.269021831666,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3461.82755399128,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3565.917066920757,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5973.163141630417,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3147800922719629,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.992070914371091,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 20.858168032167377,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3648185.0347222225,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 64383.882757713705,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4539417.291666667,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "6e0aabea38a937f64ceb8caee26fd64dd73befc4",
          "message": "fix: lockfile of the tracy features, close the viewer test (#364)\n\n* fix: commit the lockfile of the tracy features\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: close the viewer until none comes back\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: give the viewer tests steps to settle\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T09:21:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/6e0aabea38a937f64ceb8caee26fd64dd73befc4"
        },
        "date": 1793957808038,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1881.7490174360428,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 721.0527920167259,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3478.7539876656338,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3571.1579512259395,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5981.273985218442,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3152645452657118,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.9924014303722934,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 20.811664717229096,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3656990.238095238,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 64545.947918431906,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4547574.642857143,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "c7872b7274bf760a92cf0cbc229a8042fc2fd794",
          "message": "fix: keep beta quiet about the unused mimalloc (#391)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T23:56:21Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/c7872b7274bf760a92cf0cbc229a8042fc2fd794"
        },
        "date": 1794041856382,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1524.1699859392631,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 956.0323180877358,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 2295.277290553202,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 2374.476584929693,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 4289.535410115304,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.27515772213670486,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.7736648349771205,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 17.73352415083739,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3047912.0052083335,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 47854.91974494283,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3374582.5,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "210dc68a8c850a46ffb2738b228407b4215d0b5a",
          "message": "feat(core): rule model and evaluator (#81) (#416)\n\n* feat(core): rule model and evaluator (#81)\n\n[[rule]] in settings.toml with validation, a zero-allocation evaluator\n(threshold in a window, cooldown, clock injected) and the placeholder\nsubstitution of the run-a-command action, one argument per placeholder.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* fix(bench): name the rules budget like its bench\n\nbench-ab looks the budget row up by the bench name.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-08T01:07:50Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/210dc68a8c850a46ffb2738b228407b4215d0b5a"
        },
        "date": 1794129186382,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1394.2788171262396,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 419.9889183989619,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 1170.010782302999,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 1197.1751833693331,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 2061.5583317098562,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.22436945503643846,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 9.626490954177669,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 11.3921241567318,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 26156.867324064333,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2001664.230769231,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 33447.4150462772,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 2289125,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 299525,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 339550,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "cab1b6e58335300faf3b81bf0c297d4cbb03fb54",
          "message": "feat: viewer frame times over 1M rows (#84) (#431)\n\nAdd --viewer-probe-scroll: the viewer wheels its log in a real window\nand reports the app time per frame and the gaps between frames. The\nperf_driver test viewer_frames builds a 1M-row store and runs it,\nxtask perf --scenario viewer-frames turns it into the viewer.frame\nrows, and perf-baremetal.ps1 -Driver runs it on the owner PC.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T08:25:15Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/cab1b6e58335300faf3b81bf0c297d4cbb03fb54"
        },
        "date": 1794218496954,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1479.0093324678378,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 448.43007793358845,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 1246.039835453916,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 1264.091523087482,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 2199.2619793460235,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.24092340987482413,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 10.311904294067709,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 12.11931538825056,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 28083.154549216728,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2003378.7566137565,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 35929.25806405896,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 2416144.1567460317,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 295825,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 344150,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794305262831,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1971.7889791464986,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 760.2015463444433,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3496.214562846002,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3582.2083053289825,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 6426.93813466655,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.31631919518559937,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.992548689190015,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 21.69685463257374,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 36728.53397384654,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3404282.6041666665,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 62804.27546897027,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4406923.75,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 524300,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 656350,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794389953067,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1451.843443893049,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 434.86572951456924,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 1222.175262459159,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 1239.2645642761913,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 2149.8192996127586,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.2329343800826844,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 10.042078339266908,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 11.761349231371012,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 26992.12183667493,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2076100,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 34901.8063272038,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 2354288.869047619,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 298600,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 339075,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794477888934,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1961.805554291163,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 741.417498928911,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3494.8611261304063,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3573.0731313945425,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 6409.987814693473,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3166993516562085,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.987602817526729,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 21.69311415321385,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 37383.900512860564,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3473986.977040816,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 63037.12291965997,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4212201.884920634,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 576850,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 635925,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "fea96fd1d7073920800ec805bb1962809b290430",
          "message": "chore: move to Rust 1.100 (#83) (#452)\n\n* chore: move to Rust 1.100 (#83)\n\nBump the toolchain, msrv and rust-version to 1.100.0. The new\ncargo::unused_dependencies lint replaces cargo-machete, which goes\nfrom CI and the crate metadata. Builds take only crate versions that\nare a week old (registry.global-min-publish-age).\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* ci: warn only in the bench compile step (#83)\n\ncargo bench --no-run builds no test targets, so the dev-dependencies\nlook unused to the new cargo lint and the denied warning failed the\nlint job. The clippy steps build every target and still deny it.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T09:22:11Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/fea96fd1d7073920800ec805bb1962809b290430"
        },
        "date": 1794564305975,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1963.8345847788264,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 757.9585664367878,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3500.5367496449026,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3579.930500205878,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 6398.352275413381,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.31613585653482357,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.990262301624705,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 21.746265384108092,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 36377.679812215814,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 4381208.258928571,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 62443.50711641986,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4413668.402777778,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 601775,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 650000,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "188cd92f031a68fb17a8e2370fab9a4d853d7791",
          "message": "test: UI Automation smoke of the viewer (#71) (#456)\n\n* test: UI Automation smoke of the viewer (#71)\n\nAdd a UI Automation client to peekbuddy-win (feature uia) and an\ne2e test that starts the viewer, finds its window by title and walks\nthe welcome screens and the log screen by control name: Next, Skip,\nCompact, Settings, then closes the window and expects status 0.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* style: sort the peekbuddy dev-dependencies\n\ncargo-sort wants peekbuddy-win after peekbuddy-testkit.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* docs: add the contract of the UI Automation client\n\ncheck-contracts wants a Contract section on a public type that\nimplements Drop.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T16:12:43Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/188cd92f031a68fb17a8e2370fab9a4d853d7791"
        },
        "date": 1794647437213,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1444.9730689376602,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 439.20821608528496,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 1218.5522558583957,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 1242.7022386523188,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 2143.472991781329,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.233610894332254,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 10.02414083984441,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 11.916556691435176,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 27409.548373192927,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2027635.7295482296,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 35026.263314536336,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 2392029.166666667,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 303650,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 350375,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "eb945265c259cfc66e0208346b4ee4cd1e5c4bbb",
          "message": "fix: find the last walk past a burst of rows (#475)\n\nThe live lookup read the newest finish row with one filtered page and\nignored its resume cursor, so after about 50k rows since the last walk\nit found none and skipped the deleted-path check. It now follows\nresume, starts at the walk's start row and reads the rows of cancelled\nwalks too. A nightly test overflows in the middle of a burst.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-15T09:26:42Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/eb945265c259cfc66e0208346b4ee4cd1e5c4bbb"
        },
        "date": 1794735726504,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1572.7855665440875,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 926.3071941391704,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 2278.584946590732,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 2273.7605871114974,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 4655.931488078502,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.2751174875602316,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.78300717335899,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 18.098522752708313,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 29244.75755124056,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2815658.2407407407,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 48105.845827923666,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3263081.6666666665,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 405400,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 486075,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "b94413233446ea8de2a729b605ebc6a98c9fece8",
          "message": "build(deps): Bump release-plz/action from 0.5.144 to 0.5.148 (#439)\n\nBumps [release-plz/action](https://github.com/release-plz/action) from 0.5.144 to 0.5.148.\n- [Release notes](https://github.com/release-plz/action/releases)\n- [Commits](https://github.com/release-plz/action/compare/8b5bf233ecb5779f63508db7cd050e948ee77cc2...66b324fd6c72d05d2a08568492ecdd62bfb8a62b)\n\n---\nupdated-dependencies:\n- dependency-name: release-plz/action\n  dependency-version: 0.5.148\n  dependency-type: direct:production\n  update-type: version-update:semver-patch\n...\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-11-16T07:39:36Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/b94413233446ea8de2a729b605ebc6a98c9fece8"
        },
        "date": 1794826826205,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1751.0660188781017,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 1077.3273572773135,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3143.9064933415366,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3197.055935155847,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5956.866146013149,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3440931889763985,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 14.330255067108576,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 25.504154007992273,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 32003.6146226872,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 4174587.006802721,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 52093.06537806972,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3931434.375,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 593000,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 688775,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "f08653f6a71669155ebf6eee1a875aa154b5943d",
          "message": "chore: prepare v1.0.0 (#496)\n\n* chore: prepare v1.0.0\n\nVersion 1.0.0, the CHANGELOG section with the known limitations.\nRelease smoke on a release build of main: 7 of 7 checks pass; no\nnetwork connection in a 10 minute run; nightly 43950461833 green.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* chore: name the release in every changelog group\n\nmarkdownlint MD024 rejects the same group heading in two releases.\nThe group headings carry their version, in the template and in the\n1.0.0 section, so every heading in the file is unique.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-17T10:20:46Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/f08653f6a71669155ebf6eee1a875aa154b5943d"
        },
        "date": 1794913050018,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1555.8934161924524,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 947.3733531746329,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 2262.3932718361903,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 2291.1728434027564,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 4606.179825241352,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.2747895732548258,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.810045163252845,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 17.31886987342159,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 28905.253578200034,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2840774.074074074,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 48409.4209987631,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3275685.5833333335,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 368750,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 491400,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "22edfd7c71251f88f61ea679eaab09fbf60fcf23",
          "message": "ci: exclude the mutant that freezes the runner (#513)\n\nMutation shard 30 lost its runner with no log on slice 212 of the\nnightly of 2026-11-17 and slice 213 of 2026-11-18. Both slices hold\nsuspend.rs, and != for == on the owner of a thread makes\nsuspend_process suspend every thread of every other process, the\nrunner agent included. Listed with a comment like the other runaway\nmutants.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-18T09:13:40Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/22edfd7c71251f88f61ea679eaab09fbf60fcf23"
        },
        "date": 1795002285283,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1476.7158940641934,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 445.4871341126775,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 1254.3609534984425,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 1273.951784494298,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 2193.4730413896395,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.23850091434642345,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 10.220331679042975,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 12.427846468102466,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 27520.40132892216,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2222018.291170635,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 35188.16364796126,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 2446460.476190476,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 333875,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 364375,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "41ff7b00c387f20ea4409388abcb6a1202a85c83",
          "message": "test: pin the reconcile lookup of the live log (#527)\n\nThe lookup that finds where a walk reads the live rows from had 11\nmissed mutants (#365). Nine tests on a real store: the newest walk\nthat was not cancelled, a baseline, two roots (one inside the other),\nmore rows than a page, a callback that stops. Mutants on live_log.rs:\n37 caught, 0 missed; one equivalent mutant is excluded.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T10:13:14Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/41ff7b00c387f20ea4409388abcb6a1202a85c83"
        },
        "date": 1795084565654,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1551.7596616030426,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 946.4541700511713,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 2268.8257455654257,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 2301.4084829562785,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 4613.730892201631,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.27461678673019385,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.791613547411133,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 17.289412302309493,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 28783.798309178746,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3011948.765432099,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 48357.05417759135,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3249384.6666666665,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 357475,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 426100,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4c76d6913ce690540eaedb3bd273ed9a6691a40c",
          "message": "fix: read live rows from the start time of a walk (#542)\n\nThe row that starts a walk is written after live rows of events\nthat came a moment later. The next walk read the live rows after\nits id and missed entries made after their directory was listed and\ndeleted in a gap (#535). It now reads from the newest row older than\nthe start time less 10 s.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T19:49:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4c76d6913ce690540eaedb3bd273ed9a6691a40c"
        },
        "date": 1795169717154,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1500.2724321046126,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 452.2107932416331,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 1275.2171885537896,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 1287.4098816688586,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 2322.1643285751043,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.24205506373727526,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 10.463508758404206,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 12.40388297757978,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 28150.401730903617,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 2199788.541666667,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 35757.78997439875,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 2478395.595238095,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 324925,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 371250,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9675ba8dc8b3038b2e052706d79e93393c7ec2ce",
          "message": "test: show engine.log when the engine hangs (#568)\n\nThe second-engine test fails on the arm64 runner about one run in\nfour because the first engine is still running 10 s after the close.\nPut its engine.log in the failure so the next run shows where the\nshutdown stood.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T09:07:06Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/9675ba8dc8b3038b2e052706d79e93393c7ec2ce"
        },
        "date": 1795258117322,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1930.6143815809255,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 1058.4380464806027,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 2798.048418567363,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 2899.740452439316,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5308.280951838454,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.2933204787984366,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 12.811719805131368,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 22.23063434393501,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 36844.07486374905,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3996691.7261904767,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 54678.95994872581,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3990671.25,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 502800,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 661450,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "bc942ff3d705b603a67294078be324271350c287",
          "message": "docs: defer the service mode and attribution (#577)\n\nADR 0018 gets an amendment: no service mode (#79) and no process\nattribution (#80) after v1.1.0. They would ship unsigned since #74 is\nnot planned, they add a SYSTEM pipe as the first privilege boundary,\nand no user asked. ADR 0012 notes #74; the viewer reference drops the\npromise of a process column.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T22:14:19Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/bc942ff3d705b603a67294078be324271350c287"
        },
        "date": 1795342559494,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1945.1970279499274,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 750.3658272581074,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3143.4067643228395,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3208.5586880910114,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5770.33879991679,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.31872889268423377,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.993613814229185,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 20.9840646036682,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 37109.43215652327,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3971542.431972789,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 64777.413569144686,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4338895.238095239,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 555125,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 671625,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795432678459,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 2026.0376381633441,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 761.6482028659607,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3138.316177917125,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3213.665340041076,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5789.868138217029,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3198592900831509,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 4.993342704744028,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 21.730800313849663,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 37161.896486107675,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3381566.6220238097,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 62807.30709029955,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 4214490.416666667,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 581775,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 659350,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795525135503,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "filter/eval_6_names",
            "value": 1790.9704109469499,
            "unit": "ns"
          },
          {
            "name": "metrics/process_list_300",
            "value": 1069.35568494755,
            "unit": "ns"
          },
          {
            "name": "notify_parse/extended/64kib",
            "value": 3117.7460983837527,
            "unit": "ns"
          },
          {
            "name": "notify_parse/full/64kib",
            "value": 3183.2823732482807,
            "unit": "ns"
          },
          {
            "name": "notify_parse/plain/64kib",
            "value": 5826.628703374759,
            "unit": "ns"
          },
          {
            "name": "perf/counters_event",
            "value": 0.3439934250480453,
            "unit": "ns"
          },
          {
            "name": "perf/milestone_record",
            "value": 14.362674872069256,
            "unit": "ns"
          },
          {
            "name": "reparse/classify_8",
            "value": 28.023865498099703,
            "unit": "ns"
          },
          {
            "name": "rules/eval_6_events_50_rules",
            "value": 32661.451861979003,
            "unit": "ns"
          },
          {
            "name": "store/batch_insert_1000",
            "value": 3760668.9455782315,
            "unit": "ns"
          },
          {
            "name": "store/intern_hit_1000",
            "value": 53703.65760192966,
            "unit": "ns"
          },
          {
            "name": "store/intern_miss_1000",
            "value": 3780347.5,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p95",
            "value": 528175,
            "unit": "ns"
          },
          {
            "name": "viewer/scroll_frame_p99",
            "value": 633525,
            "unit": "ns"
          }
        ]
      }
    ],
    "kill soak (windows-11-vs2026-arm)": [
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "d10cfdc4c1f9c66139f6ca0f2d8650f0045ee4b3",
          "message": "test: record real traces, corpus and contracts (#287)\n\nCloses #270",
          "timestamp": "2026-11-02T09:37:50Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/d10cfdc4c1f9c66139f6ca0f2d8650f0045ee4b3"
        },
        "date": 1793612436464,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 255,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4e36337144cb7c4bd3cec1cfab240129a21fdf4d",
          "message": "fix(engine): watch a lost root's remaining path (#312)\n\nPart of #277",
          "timestamp": "2026-11-03T09:23:44Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4e36337144cb7c4bd3cec1cfab240129a21fdf4d"
        },
        "date": 1793699853519,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 1111,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "37e470769e4d8524fec1b1a6b8362533a5c8c95d",
          "message": "test: exclude two sets of equivalent mutants (#332)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-04T10:53:12Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/37e470769e4d8524fec1b1a6b8362533a5c8c95d"
        },
        "date": 1793789979432,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 355,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1fb32b54db3a63e72f95019ed223b13dad455660",
          "message": "feat(viewer): fall back to wgpu without OpenGL (#349)\n\nx64 starts with glow and starts again with wgpu/DX12 when\nglow cannot start. CI runs the fallback on hosted runners.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-05T09:06:53Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/1fb32b54db3a63e72f95019ed223b13dad455660"
        },
        "date": 1793871640327,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 204,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "6e0aabea38a937f64ceb8caee26fd64dd73befc4",
          "message": "fix: lockfile of the tracy features, close the viewer test (#364)\n\n* fix: commit the lockfile of the tracy features\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: close the viewer until none comes back\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: give the viewer tests steps to settle\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T09:21:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/6e0aabea38a937f64ceb8caee26fd64dd73befc4"
        },
        "date": 1793958020039,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 53,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 405,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "c7872b7274bf760a92cf0cbc229a8042fc2fd794",
          "message": "fix: keep beta quiet about the unused mimalloc (#391)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T23:56:21Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/c7872b7274bf760a92cf0cbc229a8042fc2fd794"
        },
        "date": 1794042043399,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 53,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 506,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "210dc68a8c850a46ffb2738b228407b4215d0b5a",
          "message": "feat(core): rule model and evaluator (#81) (#416)\n\n* feat(core): rule model and evaluator (#81)\n\n[[rule]] in settings.toml with validation, a zero-allocation evaluator\n(threshold in a window, cooldown, clock injected) and the placeholder\nsubstitution of the run-a-command action, one argument per placeholder.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* fix(bench): name the rules budget like its bench\n\nbench-ab looks the budget row up by the bench name.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-08T01:07:50Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/210dc68a8c850a46ffb2738b228407b4215d0b5a"
        },
        "date": 1794129097385,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 204,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "cab1b6e58335300faf3b81bf0c297d4cbb03fb54",
          "message": "feat: viewer frame times over 1M rows (#84) (#431)\n\nAdd --viewer-probe-scroll: the viewer wheels its log in a real window\nand reports the app time per frame and the gaps between frames. The\nperf_driver test viewer_frames builds a 1M-row store and runs it,\nxtask perf --scenario viewer-frames turns it into the viewer.frame\nrows, and perf-baremetal.ps1 -Driver runs it on the owner PC.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T08:25:15Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/cab1b6e58335300faf3b81bf0c297d4cbb03fb54"
        },
        "date": 1794218356022,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 304,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794304985889,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 154,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794389870234,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 305,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794477616319,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "fea96fd1d7073920800ec805bb1962809b290430",
          "message": "chore: move to Rust 1.100 (#83) (#452)\n\n* chore: move to Rust 1.100 (#83)\n\nBump the toolchain, msrv and rust-version to 1.100.0. The new\ncargo::unused_dependencies lint replaces cargo-machete, which goes\nfrom CI and the crate metadata. Builds take only crate versions that\nare a week old (registry.global-min-publish-age).\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* ci: warn only in the bench compile step (#83)\n\ncargo bench --no-run builds no test targets, so the dev-dependencies\nlook unused to the new cargo lint and the denied warning failed the\nlint job. The clippy steps build every target and still deny it.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T09:22:11Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/fea96fd1d7073920800ec805bb1962809b290430"
        },
        "date": 1794563829241,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 204,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "188cd92f031a68fb17a8e2370fab9a4d853d7791",
          "message": "test: UI Automation smoke of the viewer (#71) (#456)\n\n* test: UI Automation smoke of the viewer (#71)\n\nAdd a UI Automation client to peekbuddy-win (feature uia) and an\ne2e test that starts the viewer, finds its window by title and walks\nthe welcome screens and the log screen by control name: Next, Skip,\nCompact, Settings, then closes the window and expects status 0.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* style: sort the peekbuddy dev-dependencies\n\ncargo-sort wants peekbuddy-win after peekbuddy-testkit.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* docs: add the contract of the UI Automation client\n\ncheck-contracts wants a Contract section on a public type that\nimplements Drop.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T16:12:43Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/188cd92f031a68fb17a8e2370fab9a4d853d7791"
        },
        "date": 1794647311093,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 253,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "eb945265c259cfc66e0208346b4ee4cd1e5c4bbb",
          "message": "fix: find the last walk past a burst of rows (#475)\n\nThe live lookup read the newest finish row with one filtered page and\nignored its resume cursor, so after about 50k rows since the last walk\nit found none and skipped the deleted-path check. It now follows\nresume, starts at the walk's start row and reads the rows of cancelled\nwalks too. A nightly test overflows in the middle of a burst.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-15T09:26:42Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/eb945265c259cfc66e0208346b4ee4cd1e5c4bbb"
        },
        "date": 1794735573934,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 254,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "b94413233446ea8de2a729b605ebc6a98c9fece8",
          "message": "build(deps): Bump release-plz/action from 0.5.144 to 0.5.148 (#439)\n\nBumps [release-plz/action](https://github.com/release-plz/action) from 0.5.144 to 0.5.148.\n- [Release notes](https://github.com/release-plz/action/releases)\n- [Commits](https://github.com/release-plz/action/compare/8b5bf233ecb5779f63508db7cd050e948ee77cc2...66b324fd6c72d05d2a08568492ecdd62bfb8a62b)\n\n---\nupdated-dependencies:\n- dependency-name: release-plz/action\n  dependency-version: 0.5.148\n  dependency-type: direct:production\n  update-type: version-update:semver-patch\n...\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-11-16T07:39:36Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/b94413233446ea8de2a729b605ebc6a98c9fece8"
        },
        "date": 1794825674341,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 855,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "f08653f6a71669155ebf6eee1a875aa154b5943d",
          "message": "chore: prepare v1.0.0 (#496)\n\n* chore: prepare v1.0.0\n\nVersion 1.0.0, the CHANGELOG section with the known limitations.\nRelease smoke on a release build of main: 7 of 7 checks pass; no\nnetwork connection in a 10 minute run; nightly 43950461833 green.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* chore: name the release in every changelog group\n\nmarkdownlint MD024 rejects the same group heading in two releases.\nThe group headings carry their version, in the template and in the\n1.0.0 section, so every heading in the file is unique.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-17T10:20:46Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/f08653f6a71669155ebf6eee1a875aa154b5943d"
        },
        "date": 1794912901571,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 254,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "22edfd7c71251f88f61ea679eaab09fbf60fcf23",
          "message": "ci: exclude the mutant that freezes the runner (#513)\n\nMutation shard 30 lost its runner with no log on slice 212 of the\nnightly of 2026-11-17 and slice 213 of 2026-11-18. Both slices hold\nsuspend.rs, and != for == on the owner of a thread makes\nsuspend_process suspend every thread of every other process, the\nrunner agent included. Listed with a comment like the other runaway\nmutants.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-18T09:13:40Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/22edfd7c71251f88f61ea679eaab09fbf60fcf23"
        },
        "date": 1795002173440,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 204,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "41ff7b00c387f20ea4409388abcb6a1202a85c83",
          "message": "test: pin the reconcile lookup of the live log (#527)\n\nThe lookup that finds where a walk reads the live rows from had 11\nmissed mutants (#365). Nine tests on a real store: the newest walk\nthat was not cancelled, a baseline, two roots (one inside the other),\nmore rows than a page, a callback that stops. Mutants on live_log.rs:\n37 caught, 0 missed; one equivalent mutant is excluded.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T10:13:14Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/41ff7b00c387f20ea4409388abcb6a1202a85c83"
        },
        "date": 1795084409653,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4c76d6913ce690540eaedb3bd273ed9a6691a40c",
          "message": "fix: read live rows from the start time of a walk (#542)\n\nThe row that starts a walk is written after live rows of events\nthat came a moment later. The next walk read the live rows after\nits id and missed entries made after their directory was listed and\ndeleted in a gap (#535). It now reads from the newest row older than\nthe start time less 10 s.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T19:49:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4c76d6913ce690540eaedb3bd273ed9a6691a40c"
        },
        "date": 1795169630419,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 522,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9675ba8dc8b3038b2e052706d79e93393c7ec2ce",
          "message": "test: show engine.log when the engine hangs (#568)\n\nThe second-engine test fails on the arm64 runner about one run in\nfour because the first engine is still running 10 s after the close.\nPut its engine.log in the failure so the next run shows where the\nshutdown stood.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T09:07:06Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/9675ba8dc8b3038b2e052706d79e93393c7ec2ce"
        },
        "date": 1795257158744,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "bc942ff3d705b603a67294078be324271350c287",
          "message": "docs: defer the service mode and attribution (#577)\n\nADR 0018 gets an amendment: no service mode (#79) and no process\nattribution (#80) after v1.1.0. They would ship unsigned since #74 is\nnot planned, they add a SYSTEM pipe as the first privilege boundary,\nand no user asked. ADR 0012 notes #74; the viewer reference drops the\npromise of a process column.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T22:14:19Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/bc942ff3d705b603a67294078be324271350c287"
        },
        "date": 1795341475744,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 253,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795431481272,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 154,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795524074328,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 204,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795606286689,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 103,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 254,
            "unit": "ms"
          }
        ]
      }
    ],
    "kill soak (windows-2025)": [
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "d10cfdc4c1f9c66139f6ca0f2d8650f0045ee4b3",
          "message": "test: record real traces, corpus and contracts (#287)\n\nCloses #270",
          "timestamp": "2026-11-02T09:37:50Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/d10cfdc4c1f9c66139f6ca0f2d8650f0045ee4b3"
        },
        "date": 1793612768552,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 353,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4e36337144cb7c4bd3cec1cfab240129a21fdf4d",
          "message": "fix(engine): watch a lost root's remaining path (#312)\n\nPart of #277",
          "timestamp": "2026-11-03T09:23:44Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4e36337144cb7c4bd3cec1cfab240129a21fdf4d"
        },
        "date": 1793699466856,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 52,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 555,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4e8f1ad0cffccf28d49a35b70acbc63c732d39b6",
          "message": "test(store): pin the log size and the pragmas (#334)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-04T10:59:55Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4e8f1ad0cffccf28d49a35b70acbc63c732d39b6"
        },
        "date": 1793790253099,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 52,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1fb32b54db3a63e72f95019ed223b13dad455660",
          "message": "feat(viewer): fall back to wgpu without OpenGL (#349)\n\nx64 starts with glow and starts again with wgpu/DX12 when\nglow cannot start. CI runs the fallback on hosted runners.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-05T09:06:53Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/1fb32b54db3a63e72f95019ed223b13dad455660"
        },
        "date": 1793871577924,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "6e0aabea38a937f64ceb8caee26fd64dd73befc4",
          "message": "fix: lockfile of the tracy features, close the viewer test (#364)\n\n* fix: commit the lockfile of the tracy features\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: close the viewer until none comes back\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: give the viewer tests steps to settle\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T09:21:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/6e0aabea38a937f64ceb8caee26fd64dd73befc4"
        },
        "date": 1793957804208,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 52,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 253,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "c7872b7274bf760a92cf0cbc229a8042fc2fd794",
          "message": "fix: keep beta quiet about the unused mimalloc (#391)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T23:56:21Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/c7872b7274bf760a92cf0cbc229a8042fc2fd794"
        },
        "date": 1794041828099,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 253,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "210dc68a8c850a46ffb2738b228407b4215d0b5a",
          "message": "feat(core): rule model and evaluator (#81) (#416)\n\n* feat(core): rule model and evaluator (#81)\n\n[[rule]] in settings.toml with validation, a zero-allocation evaluator\n(threshold in a window, cooldown, clock injected) and the placeholder\nsubstitution of the run-a-command action, one argument per placeholder.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* fix(bench): name the rules budget like its bench\n\nbench-ab looks the budget row up by the bench name.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-08T01:07:50Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/210dc68a8c850a46ffb2738b228407b4215d0b5a"
        },
        "date": 1794129013992,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 202,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "cab1b6e58335300faf3b81bf0c297d4cbb03fb54",
          "message": "feat: viewer frame times over 1M rows (#84) (#431)\n\nAdd --viewer-probe-scroll: the viewer wheels its log in a real window\nand reports the app time per frame and the gaps between frames. The\nperf_driver test viewer_frames builds a 1M-row store and runs it,\nxtask perf --scenario viewer-frames turns it into the viewer.frame\nrows, and perf-baremetal.ps1 -Driver runs it on the owner PC.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T08:25:15Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/cab1b6e58335300faf3b81bf0c297d4cbb03fb54"
        },
        "date": 1794218363327,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 254,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794304932717,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794389818049,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 857,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794477647953,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 254,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "fea96fd1d7073920800ec805bb1962809b290430",
          "message": "chore: move to Rust 1.100 (#83) (#452)\n\n* chore: move to Rust 1.100 (#83)\n\nBump the toolchain, msrv and rust-version to 1.100.0. The new\ncargo::unused_dependencies lint replaces cargo-machete, which goes\nfrom CI and the crate metadata. Builds take only crate versions that\nare a week old (registry.global-min-publish-age).\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* ci: warn only in the bench compile step (#83)\n\ncargo bench --no-run builds no test targets, so the dev-dependencies\nlook unused to the new cargo lint and the denied warning failed the\nlint job. The clippy steps build every target and still deny it.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T09:22:11Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/fea96fd1d7073920800ec805bb1962809b290430"
        },
        "date": 1794563944774,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 807,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "188cd92f031a68fb17a8e2370fab9a4d853d7791",
          "message": "test: UI Automation smoke of the viewer (#71) (#456)\n\n* test: UI Automation smoke of the viewer (#71)\n\nAdd a UI Automation client to peekbuddy-win (feature uia) and an\ne2e test that starts the viewer, finds its window by title and walks\nthe welcome screens and the log screen by control name: Next, Skip,\nCompact, Settings, then closes the window and expects status 0.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* style: sort the peekbuddy dev-dependencies\n\ncargo-sort wants peekbuddy-win after peekbuddy-testkit.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* docs: add the contract of the UI Automation client\n\ncheck-contracts wants a Contract section on a public type that\nimplements Drop.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T16:12:43Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/188cd92f031a68fb17a8e2370fab9a4d853d7791"
        },
        "date": 1794647267334,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 253,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "42fc59e8d8fb2a8535011761da7b84b704c312fa",
          "message": "test: check that PeekBuddy makes no network call (#472)\n\n* test: check that PeekBuddy makes no network call\n\ncheck-release now fails when the exe names a network library, and\nscripts/owner/no-network.ps1 watches the sockets of a real run of\nthe engine and the viewer for ten minutes. Both back the v1 promise.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* fix: write the NULs of a test literal as escapes\n\nThe test string held real NUL bytes, which editorconfig-checker read\nas a binary file. Spell them as backslash zero.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-14T21:54:41Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/42fc59e8d8fb2a8535011761da7b84b704c312fa"
        },
        "date": 1794735514435,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 404,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "b94413233446ea8de2a729b605ebc6a98c9fece8",
          "message": "build(deps): Bump release-plz/action from 0.5.144 to 0.5.148 (#439)\n\nBumps [release-plz/action](https://github.com/release-plz/action) from 0.5.144 to 0.5.148.\n- [Release notes](https://github.com/release-plz/action/releases)\n- [Commits](https://github.com/release-plz/action/compare/8b5bf233ecb5779f63508db7cd050e948ee77cc2...66b324fd6c72d05d2a08568492ecdd62bfb8a62b)\n\n---\nupdated-dependencies:\n- dependency-name: release-plz/action\n  dependency-version: 0.5.148\n  dependency-type: direct:production\n  update-type: version-update:semver-patch\n...\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-11-16T07:39:36Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/b94413233446ea8de2a729b605ebc6a98c9fece8"
        },
        "date": 1794825701409,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 305,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "f08653f6a71669155ebf6eee1a875aa154b5943d",
          "message": "chore: prepare v1.0.0 (#496)\n\n* chore: prepare v1.0.0\n\nVersion 1.0.0, the CHANGELOG section with the known limitations.\nRelease smoke on a release build of main: 7 of 7 checks pass; no\nnetwork connection in a 10 minute run; nightly 43950461833 green.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* chore: name the release in every changelog group\n\nmarkdownlint MD024 rejects the same group heading in two releases.\nThe group headings carry their version, in the template and in the\n1.0.0 section, so every heading in the file is unique.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-17T10:20:46Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/f08653f6a71669155ebf6eee1a875aa154b5943d"
        },
        "date": 1794912904953,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 303,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "22edfd7c71251f88f61ea679eaab09fbf60fcf23",
          "message": "ci: exclude the mutant that freezes the runner (#513)\n\nMutation shard 30 lost its runner with no log on slice 212 of the\nnightly of 2026-11-17 and slice 213 of 2026-11-18. Both slices hold\nsuspend.rs, and != for == on the owner of a thread makes\nsuspend_process suspend every thread of every other process, the\nrunner agent included. Listed with a comment like the other runaway\nmutants.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-18T09:13:40Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/22edfd7c71251f88f61ea679eaab09fbf60fcf23"
        },
        "date": 1795002093978,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 253,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "41ff7b00c387f20ea4409388abcb6a1202a85c83",
          "message": "test: pin the reconcile lookup of the live log (#527)\n\nThe lookup that finds where a walk reads the live rows from had 11\nmissed mutants (#365). Nine tests on a real store: the newest walk\nthat was not cancelled, a baseline, two roots (one inside the other),\nmore rows than a page, a callback that stops. Mutants on live_log.rs:\n37 caught, 0 missed; one equivalent mutant is excluded.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T10:13:14Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/41ff7b00c387f20ea4409388abcb6a1202a85c83"
        },
        "date": 1795084423937,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4c76d6913ce690540eaedb3bd273ed9a6691a40c",
          "message": "fix: read live rows from the start time of a walk (#542)\n\nThe row that starts a walk is written after live rows of events\nthat came a moment later. The next walk read the live rows after\nits id and missed entries made after their directory was listed and\ndeleted in a gap (#535). It now reads from the newest row older than\nthe start time less 10 s.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T19:49:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4c76d6913ce690540eaedb3bd273ed9a6691a40c"
        },
        "date": 1795169611098,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 203,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9675ba8dc8b3038b2e052706d79e93393c7ec2ce",
          "message": "test: show engine.log when the engine hangs (#568)\n\nThe second-engine test fails on the arm64 runner about one run in\nfour because the first engine is still running 10 s after the close.\nPut its engine.log in the failure so the next run shows where the\nshutdown stood.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T09:07:06Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/9675ba8dc8b3038b2e052706d79e93393c7ec2ce"
        },
        "date": 1795257130134,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 51,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 202,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "bc942ff3d705b603a67294078be324271350c287",
          "message": "docs: defer the service mode and attribution (#577)\n\nADR 0018 gets an amendment: no service mode (#79) and no process\nattribution (#80) after v1.1.0. They would ship unsigned since #74 is\nnot planned, they add a SYSTEM pipe as the first privilege boundary,\nand no user asked. ADR 0012 notes #74; the viewer reference drops the\npromise of a process column.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T22:14:19Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/bc942ff3d705b603a67294078be324271350c287"
        },
        "date": 1795341498152,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 153,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795431501529,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 153,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795524136307,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 303,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795606311312,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "durability.restart_to_ready.p50",
            "value": 102,
            "unit": "ms"
          },
          {
            "name": "durability.restart_to_ready.max",
            "value": 667,
            "unit": "ms"
          }
        ]
      }
    ],
    "query benches (windows-2025)": [
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4e36337144cb7c4bd3cec1cfab240129a21fdf4d",
          "message": "fix(engine): watch a lost root's remaining path (#312)\n\nPart of #277",
          "timestamp": "2026-11-03T09:23:44Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4e36337144cb7c4bd3cec1cfab240129a21fdf4d"
        },
        "date": 1793699580891,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.filtered_first_1m",
            "value": 7140763.162393162,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 33005336.458333336,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2383613.325358852,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3712569300,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 10191823.888888888,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4e8f1ad0cffccf28d49a35b70acbc63c732d39b6",
          "message": "test(store): pin the log size and the pragmas (#334)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-04T10:59:55Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4e8f1ad0cffccf28d49a35b70acbc63c732d39b6"
        },
        "date": 1793793284192,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.filtered_first_1m",
            "value": 6733415.384615384,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 30689323.333333336,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2303329.333333333,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3555078950,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9230259.523809522,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "1fb32b54db3a63e72f95019ed223b13dad455660",
          "message": "feat(viewer): fall back to wgpu without OpenGL (#349)\n\nx64 starts with glow and starts again with wgpu/DX12 when\nglow cannot start. CI runs the fallback on hosted runners.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-05T09:06:53Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/1fb32b54db3a63e72f95019ed223b13dad455660"
        },
        "date": 1793871551933,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.filtered_first_1m",
            "value": 5970631.851851853,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 19668742.013888888,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 1300193.2291666665,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2045140550,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 6073766.785714285,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "6e0aabea38a937f64ceb8caee26fd64dd73befc4",
          "message": "fix: lockfile of the tracy features, close the viewer test (#364)\n\n* fix: commit the lockfile of the tracy features\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: close the viewer until none comes back\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* test: give the viewer tests steps to settle\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T09:21:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/6e0aabea38a937f64ceb8caee26fd64dd73befc4"
        },
        "date": 1793963069195,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.filtered_first_1m",
            "value": 6805420.68452381,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 35932394.44444445,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2354371.5,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3675438000,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9532334.210526317,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "c7872b7274bf760a92cf0cbc229a8042fc2fd794",
          "message": "fix: keep beta quiet about the unused mimalloc (#391)\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-06T23:56:21Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/c7872b7274bf760a92cf0cbc229a8042fc2fd794"
        },
        "date": 1794042026068,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.filtered_first_1m",
            "value": 6679149.149659865,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 32043315.128205128,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2362695.757575758,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3525181200,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9223110.016835017,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "210dc68a8c850a46ffb2738b228407b4215d0b5a",
          "message": "feat(core): rule model and evaluator (#81) (#416)\n\n* feat(core): rule model and evaluator (#81)\n\n[[rule]] in settings.toml with validation, a zero-allocation evaluator\n(threshold in a window, cooldown, clock injected) and the placeholder\nsubstitution of the run-a-command action, one argument per placeholder.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* fix(bench): name the rules budget like its bench\n\nbench-ab looks the budget row up by the bench name.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-08T01:07:50Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/210dc68a8c850a46ffb2738b228407b4215d0b5a"
        },
        "date": 1794129079344,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.filtered_first_1m",
            "value": 6028164.047619048,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 20218992.112299465,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 1285963.7626262626,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2128864900,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 6026490.333333334,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "cab1b6e58335300faf3b81bf0c297d4cbb03fb54",
          "message": "feat: viewer frame times over 1M rows (#84) (#431)\n\nAdd --viewer-probe-scroll: the viewer wheels its log in a real window\nand reports the app time per frame and the gaps between frames. The\nperf_driver test viewer_frames builds a 1M-row store and runs it,\nxtask perf --scenario viewer-frames turns it into the viewer.frame\nrows, and perf-baremetal.ps1 -Driver runs it on the owner PC.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T08:25:15Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/cab1b6e58335300faf3b81bf0c297d4cbb03fb54"
        },
        "date": 1794218541948,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1586226150,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 6636786.30952381,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 31177619.11764706,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2316786.9444444445,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3468794900,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9170511.666666668,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794305209536,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1684680700,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 6751202.678571428,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 37441395.39473684,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2804837.7314814813,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3451822950,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9602113.888888888,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794390036871,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1969566100,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 7149235.225885225,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 32584204.385964915,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2108053.856749311,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3760890350,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9959849.264705881,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "433163889d3a3019beb307566843cb941a742fd4",
          "message": "test: examine viewer timeline and worker mutants (#441)\n\nTake timeline.rs and worker.rs off the mutants exclude list. A full\nlocal run gave 14 missed of 195 judged (93 %), the survivors being\nequivalent boundary mutants and the tracy-only plot value, which is\nexcluded by name.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-09T22:58:25Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/433163889d3a3019beb307566843cb941a742fd4"
        },
        "date": 1794477827252,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1843076450,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 6682511.30952381,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 33760361.554621845,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2338684.9523809524,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3549249300,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9621817.222222222,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "fea96fd1d7073920800ec805bb1962809b290430",
          "message": "chore: move to Rust 1.100 (#83) (#452)\n\n* chore: move to Rust 1.100 (#83)\n\nBump the toolchain, msrv and rust-version to 1.100.0. The new\ncargo::unused_dependencies lint replaces cargo-machete, which goes\nfrom CI and the crate metadata. Builds take only crate versions that\nare a week old (registry.global-min-publish-age).\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* ci: warn only in the bench compile step (#83)\n\ncargo bench --no-run builds no test targets, so the dev-dependencies\nlook unused to the new cargo lint and the denied warning failed the\nlint job. The clippy steps build every target and still deny it.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T09:22:11Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/fea96fd1d7073920800ec805bb1962809b290430"
        },
        "date": 1794563977237,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1800687200,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 6873229.761904762,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 30673142.410714284,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2381315.4545454546,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3505074250,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9214184.31372549,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "188cd92f031a68fb17a8e2370fab9a4d853d7791",
          "message": "test: UI Automation smoke of the viewer (#71) (#456)\n\n* test: UI Automation smoke of the viewer (#71)\n\nAdd a UI Automation client to peekbuddy-win (feature uia) and an\ne2e test that starts the viewer, finds its window by title and walks\nthe welcome screens and the log screen by control name: Next, Skip,\nCompact, Settings, then closes the window and expects status 0.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* style: sort the peekbuddy dev-dependencies\n\ncargo-sort wants peekbuddy-win after peekbuddy-testkit.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* docs: add the contract of the UI Automation client\n\ncheck-contracts wants a Contract section on a public type that\nimplements Drop.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-13T16:12:43Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/188cd92f031a68fb17a8e2370fab9a4d853d7791"
        },
        "date": 1794647533020,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1641438800,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 6629568.75,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 35292744.03409091,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2634658.67003367,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3623197600,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 10494990.68627451,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "eb945265c259cfc66e0208346b4ee4cd1e5c4bbb",
          "message": "fix: find the last walk past a burst of rows (#475)\n\nThe live lookup read the newest finish row with one filtered page and\nignored its resume cursor, so after about 50k rows since the last walk\nit found none and skipped the deleted-path check. It now follows\nresume, starts at the walk's start row and reads the rows of cancelled\nwalks too. A nightly test overflows in the middle of a burst.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-15T09:26:42Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/eb945265c259cfc66e0208346b4ee4cd1e5c4bbb"
        },
        "date": 1794735824202,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1879571800,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 7286211.805555556,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 33401931.81818182,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2399148.083333333,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3739662300,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 10452612.121212121,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "b94413233446ea8de2a729b605ebc6a98c9fece8",
          "message": "build(deps): Bump release-plz/action from 0.5.144 to 0.5.148 (#439)\n\nBumps [release-plz/action](https://github.com/release-plz/action) from 0.5.144 to 0.5.148.\n- [Release notes](https://github.com/release-plz/action/releases)\n- [Commits](https://github.com/release-plz/action/compare/8b5bf233ecb5779f63508db7cd050e948ee77cc2...66b324fd6c72d05d2a08568492ecdd62bfb8a62b)\n\n---\nupdated-dependencies:\n- dependency-name: release-plz/action\n  dependency-version: 0.5.148\n  dependency-type: direct:production\n  update-type: version-update:semver-patch\n...\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-11-16T07:39:36Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/b94413233446ea8de2a729b605ebc6a98c9fece8"
        },
        "date": 1794825854295,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1595682100,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 6613766.666666667,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 31501716.42857143,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2282187.5588235296,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3451382350,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9166536.458333332,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "f08653f6a71669155ebf6eee1a875aa154b5943d",
          "message": "chore: prepare v1.0.0 (#496)\n\n* chore: prepare v1.0.0\n\nVersion 1.0.0, the CHANGELOG section with the known limitations.\nRelease smoke on a release build of main: 7 of 7 checks pass; no\nnetwork connection in a 10 minute run; nightly 43950461833 green.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n* chore: name the release in every changelog group\n\nmarkdownlint MD024 rejects the same group heading in two releases.\nThe group headings carry their version, in the template and in the\n1.0.0 section, so every heading in the file is unique.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-17T10:20:46Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/f08653f6a71669155ebf6eee1a875aa154b5943d"
        },
        "date": 1794913085279,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1693293650,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 6698844.196428571,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 31164546.153846152,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2312524.166666667,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 3587896400,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9961154.678362574,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "22edfd7c71251f88f61ea679eaab09fbf60fcf23",
          "message": "ci: exclude the mutant that freezes the runner (#513)\n\nMutation shard 30 lost its runner with no log on slice 212 of the\nnightly of 2026-11-17 and slice 213 of 2026-11-18. Both slices hold\nsuspend.rs, and != for == on the owner of a thread makes\nsuspend_process suspend every thread of every other process, the\nrunner agent included. Listed with a comment like the other runaway\nmutants.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-18T09:13:40Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/22edfd7c71251f88f61ea679eaab09fbf60fcf23"
        },
        "date": 1795002247883,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1434674350,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5035228.50877193,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 23879902.631578945,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 1608376.875,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2639379500,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 7116749.107142857,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "41ff7b00c387f20ea4409388abcb6a1202a85c83",
          "message": "test: pin the reconcile lookup of the live log (#527)\n\nThe lookup that finds where a walk reads the live rows from had 11\nmissed mutants (#365). Nine tests on a real store: the newest walk\nthat was not cancelled, a baseline, two roots (one inside the other),\nmore rows than a page, a callback that stops. Mutants on live_log.rs:\n37 caught, 0 missed; one equivalent mutant is excluded.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T10:13:14Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/41ff7b00c387f20ea4409388abcb6a1202a85c83"
        },
        "date": 1795084736141,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1525956200,
            "unit": "ns"
          },
          {
            "name": "query.export_1m_rows",
            "value": 4028584850,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5257887.962962963,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 27132016.729323305,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 1678270.393668831,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2860377850,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 8021576.666666666,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4c76d6913ce690540eaedb3bd273ed9a6691a40c",
          "message": "fix: read live rows from the start time of a walk (#542)\n\nThe row that starts a walk is written after live rows of events\nthat came a moment later. The next walk read the live rows after\nits id and missed entries made after their directory was listed and\ndeleted in a gap (#535). It now reads from the newest row older than\nthe start time less 10 s.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-19T19:49:03Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/4c76d6913ce690540eaedb3bd273ed9a6691a40c"
        },
        "date": 1795169654002,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1046565300,
            "unit": "ns"
          },
          {
            "name": "query.export_1m_rows",
            "value": 2779506950,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5651106.339285715,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 18741671.719457015,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 1200794.5244690673,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 1956777750,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 5217808.333333334,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "9675ba8dc8b3038b2e052706d79e93393c7ec2ce",
          "message": "test: show engine.log when the engine hangs (#568)\n\nThe second-engine test fails on the arm64 runner about one run in\nfour because the first engine is still running 10 s after the close.\nPut its engine.log in the failure so the next run shows where the\nshutdown stood.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T09:07:06Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/9675ba8dc8b3038b2e052706d79e93393c7ec2ce"
        },
        "date": 1795257404548,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 2113164550,
            "unit": "ns"
          },
          {
            "name": "query.export_1m_rows",
            "value": 3636696650,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5892375.208333334,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 37551073.20261438,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 1902262.2727272727,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2698976500,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 10432047.542735044,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "bc942ff3d705b603a67294078be324271350c287",
          "message": "docs: defer the service mode and attribution (#577)\n\nADR 0018 gets an amendment: no service mode (#79) and no process\nattribution (#80) after v1.1.0. They would ship unsigned since #74 is\nnot planned, they add a SYSTEM pipe as the first privilege boundary,\nand no user asked. ADR 0012 notes #74; the viewer reference drops the\npromise of a process column.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-21T22:14:19Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/bc942ff3d705b603a67294078be324271350c287"
        },
        "date": 1795341612212,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1479900700,
            "unit": "ns"
          },
          {
            "name": "query.export_1m_rows",
            "value": 2887412350,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5217795.833333334,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 25310478.333333336,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 1506937.9166666665,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 1960794000,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 7135877.859477124,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795431700896,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1571318150,
            "unit": "ns"
          },
          {
            "name": "query.export_1m_rows",
            "value": 3453579450,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5176196.296296297,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 31279782.5,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2217266.7464114833,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2662570550,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9362357.142857142,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795524340294,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1846030400,
            "unit": "ns"
          },
          {
            "name": "query.export_1m_rows",
            "value": 3773006100,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5349644.117647059,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 37165918.65079366,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2557546.2280701753,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2881575700,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 10235227.267156862,
            "unit": "ns"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Matthias Eggert",
            "username": "besessener",
            "email": "8039350+besessener@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "24558351bf7f4762844f3e131f80877de8793ff3",
          "message": "feat(viewer): hash a row's file on demand (#591)\n\n* feat(viewer): hash a row's file on demand\n\nThe details pane gets a button for the SHA-256 of the file now (#85):\nthe file at the row's path is read on its own thread, 64 KiB at a\ntime, cancellable, with every share mode, through the system's\nSHA-256. A cloud placeholder is not read, so a hash never downloads a\nfile.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n* test: compare the hashed path as text\n\nOff Windows a backslash is no path separator, so Path::ends_with\nnever matched the file name on the Linux job.\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-22T22:36:22Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/24558351bf7f4762844f3e131f80877de8793ff3"
        },
        "date": 1795606522159,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "query.changes_since_1m",
            "value": 1883484400,
            "unit": "ns"
          },
          {
            "name": "query.export_1m_rows",
            "value": 3820625900,
            "unit": "ns"
          },
          {
            "name": "query.filtered_first_1m",
            "value": 5365531.092436975,
            "unit": "ns"
          },
          {
            "name": "query.fts_lookup_10m",
            "value": 32197205.35714286,
            "unit": "ns"
          },
          {
            "name": "query.page_keyset_10m",
            "value": 2185572.510822511,
            "unit": "ns"
          },
          {
            "name": "query.regex_post_filter_1m",
            "value": 2757022550,
            "unit": "ns"
          },
          {
            "name": "query.subtree_resolve_10m",
            "value": 9200917.676767677,
            "unit": "ns"
          }
        ]
      }
    ]
  }
}