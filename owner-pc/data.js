window.BENCHMARK_DATA = {
  "lastUpdate": 1794176893209,
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
      }
    ]
  }
}