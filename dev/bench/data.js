window.BENCHMARK_DATA = {
  "lastUpdate": 1794389954960,
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
      }
    ]
  }
}