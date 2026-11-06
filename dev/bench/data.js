window.BENCHMARK_DATA = {
  "lastUpdate": 1793963071485,
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
      }
    ]
  }
}