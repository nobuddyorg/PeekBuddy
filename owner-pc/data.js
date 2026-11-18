window.BENCHMARK_DATA = {
  "lastUpdate": 1794985351551,
  "repoUrl": "https://github.com/nobuddyorg/PeekBuddy",
  "entries": {
    "owner PC": [
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
          "id": "10deecdafb6b8d02604a2ef5c0fa06e776fbac13",
          "message": "feat: publish owner PC reports to gh-pages (#84) (#429)\n\nAdd the owner-pc workflow: a report committed under perf/owner-runs\non main becomes one point per budget row in the owner-pc series.\nDocument that a published report keeps the default 60 s idle window.\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-08T22:14:13Z",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/10deecdafb6b8d02604a2ef5c0fa06e776fbac13"
        },
        "date": 1794176892633,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "engine.handles.stable",
            "value": 0,
            "unit": "count"
          },
          {
            "name": "engine.idle.cpu",
            "value": 0,
            "unit": "%"
          },
          {
            "name": "engine.idle.wakes",
            "value": 7.994676877645634,
            "unit": "per min"
          },
          {
            "name": "engine.private_bytes.burst",
            "value": 221.71875,
            "unit": "MB"
          },
          {
            "name": "engine.private_bytes.idle",
            "value": 2.953125,
            "unit": "MB"
          },
          {
            "name": "engine.ready.warm",
            "value": 157.474,
            "unit": "ms"
          },
          {
            "name": "engine.startup.cpu",
            "value": 70.3125,
            "unit": "ms"
          },
          {
            "name": "engine.startup.disk_read",
            "value": 0.01648,
            "unit": "MB"
          },
          {
            "name": "event.latency.p50",
            "value": 446.526,
            "unit": "ms"
          },
          {
            "name": "event.latency.p99",
            "value": 8852.9524,
            "unit": "ms"
          },
          {
            "name": "ingest.undetected_loss",
            "value": 0,
            "unit": "count"
          },
          {
            "name": "viewer.first_frame.median",
            "value": 372.2215,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "8039350+besessener@users.noreply.github.com",
            "name": "Matthias Eggert",
            "username": "besessener"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "5a48f4ce10123d61fa90e428435cebd4a4cabe83",
          "message": "perf: owner PC run of v1.0.0 (#511)\n\nperf-baremetal.ps1 on the owner PC (Windows 11 26300, Intel\ngraphics, Defender real-time on) against the published x64 exe,\nwith the harness of CI run 44196527589 (attested). Warm ready 104\nms, startup CPU 62.5 ms, idle 0 wakes and 2.9 MB, handles stable,\n1M-row viewer frames p95 1.2 ms, first frame 153 ms. latency-10k:\nthe generator reaches 3.2k events/s, p50 108 ms, p99 1.7 s, peak\n153 MB (v1.0.0, before #509).\n\nCo-authored-by: Matthias Eggert <dem.matze@gmail.com>\nCo-authored-by: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-11-18T06:47:59Z",
          "tree_id": "c4b970c7b62e53d41f32d2c3b23b2d286847b58d",
          "url": "https://github.com/nobuddyorg/PeekBuddy/commit/5a48f4ce10123d61fa90e428435cebd4a4cabe83"
        },
        "date": 1794985350622,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "engine.handles.stable",
            "value": 0,
            "unit": "count"
          },
          {
            "name": "engine.idle.cpu",
            "value": 0,
            "unit": "%"
          },
          {
            "name": "engine.idle.wakes",
            "value": 7.994676877645634,
            "unit": "per min"
          },
          {
            "name": "engine.private_bytes.burst",
            "value": 221.71875,
            "unit": "MB"
          },
          {
            "name": "engine.private_bytes.idle",
            "value": 2.953125,
            "unit": "MB"
          },
          {
            "name": "engine.ready.warm",
            "value": 157.474,
            "unit": "ms"
          },
          {
            "name": "engine.startup.cpu",
            "value": 70.3125,
            "unit": "ms"
          },
          {
            "name": "engine.startup.disk_read",
            "value": 0.01648,
            "unit": "MB"
          },
          {
            "name": "event.latency.p50",
            "value": 446.526,
            "unit": "ms"
          },
          {
            "name": "event.latency.p99",
            "value": 8852.9524,
            "unit": "ms"
          },
          {
            "name": "ingest.undetected_loss",
            "value": 0,
            "unit": "count"
          },
          {
            "name": "viewer.first_frame.median",
            "value": 372.2215,
            "unit": "ms"
          }
        ]
      }
    ]
  }
}