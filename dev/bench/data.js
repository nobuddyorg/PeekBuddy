window.BENCHMARK_DATA = {
  "lastUpdate": 1793612439284,
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
      }
    ]
  }
}