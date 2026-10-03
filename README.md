# Torrent

An Ice Commander plugin that opens a `.torrent` as a folder and downloads what
you choose from it, using [librqbit](https://github.com/ikatson/rqbit).

## What it does

- Claims `.torrent`. Walking into one lists the files inside it as a directory
  tree, with two extra columns: **Fetch**, a tick box per row, and **Status**.
- The torrent is read through the host's `fs_*` calls, and the host is asked for
  its local path (`fs_local_path`); without one the mount is refused. Downloads
  go to `<name>.torrent-data/` beside the `.torrent`, and the choice of files to
  `<name>.torrent-state.json` beside it (`info_hash`, `selected`, `done`).
- Ticking or unticking a row writes the state file straight away and, if the
  torrent is running, changes its file selection. A directory row stands for
  every file under it. Ticking the first file starts the torrent; unticking the
  last one pauses it.
- Opening a torrent starts it if anything is ticked: it connects, checks what is
  already on disk, seeds it and fetches the rest. A torrent with no state file
  has nothing ticked and stays off.
- **Status** shows the percentage (rounded down, never 100 before the file is
  complete), then **Ready**. Unticked rows and directories show nothing. Each
  listing of a running torrent writes the finished files to `done` (only when
  that changes), so a torrent that is not running still shows **Ready** for
  them. Sizes on disk prove nothing: the engine lays every file out at full
  size before fetching it.
- The panel's default toolbar is hidden inside a torrent. The plugin's own
  buttons:
  - **All files**, **Downloaded**, **Peers** — three views, one always pressed.
    Downloaded and Peers are disabled while the torrent is not running, unless
    already shown.
    Downloaded is a flat list of the files the engine reports complete. Peers
    lists the engine's peers with **State**, **Pieces** and **Errors**; opening a
    peer shows its details as text.
  - **Fetch** / **Leave** — add the panel selection to the files to fetch, or
    take it off; a directory means everything under it.
  - **Throw away** — deletes the selected files from the data folder, unticks
    them and removes them from `done`. The filesystem's `remove` does the same.
    Fetch, Leave and Throw away are disabled in the Peers view.
  - **Clean up** — asks first, then removes the torrent from the session and
    deletes the data folder and the state file, and the `.torrent` itself if
    that switch is on (off by default). Disabled while the torrent is running.
- Writing, renaming and creating folders are refused with "a torrent cannot be
  written to, only downloaded". The filesystem is not reported read-only, so
  deleting still works.
- While any torrent is running, a header button shows the total speeds
  (`↓ 1.5 MB/s  ↑ 0`). It opens **Downloads**: one row per running torrent with
  its name, percentage, download speed and a **Stop** button that pauses it. The
  button is hidden when nothing runs.
- A torrent keeps downloading after every panel has left it. The panel listing,
  the header and the Downloads dialogue refresh once a second while something
  runs; the ticker thread stops after five quiet seconds.
- The session runs on the plugin's own Tokio runtime. On unload the plugin stops
  the ticker, the session and the runtime, giving each up to five seconds.
- Phrases ship in 15 languages and are registered with the host at start-up.
  Cell text is translated by the plugin in the language the host reports at
  init; a missing phrase falls back to English, then to the key.

The host must provide the plugin table up to `fs_local_path`; an older host is
refused with `IC_ERR_HOST_TOO_OLD`.

## Building

```sh
./build.sh          # release build, libraries collected into bin/
./test.sh           # cargo test --workspace
./deploy-local.sh   # copies bin/* into the plugin folder (IC_PLUGIN_DIR overrides it)
```

`ic-plugin-api` is fetched from its git repository. Build output goes to
`bin/target`. After deploying, enable the plugin in **Settings → Plugins** and
restart. The version lives in `package.json`; `npm run gen-version` writes it
into `version.rs`.

`cargo test -- --ignored` runs the tests that start a real librqbit session.
Those that need a real torrent read its path from `IC_TORRENT_FILE` and do
nothing when it is unset; the download test also needs the network and peers.

## Known limitations

- A torrent opened from a non-local filesystem gets its data folder beside the
  host's local copy of the `.torrent`, not beside the original.
- The engine session is not persisted: after a restart a torrent starts again
  only when it is opened.
- **Stop** in Downloads pauses the torrent and leaves it in the session, out of
  the list. Opening the torrent again resumes it; only Clean up or quitting
  removes it.
- The engine identifies a torrent by its info hash, so the same torrent saved in
  two places cannot be open at once: the second is refused as already open.
- A deleted file is not re-checked by the engine, which still believes it has
  the pieces; ticking it again does not fetch it until the torrent is next added
  to the session.
- A file smaller than a piece shares that piece with its neighbours. Deleting it
  while a neighbour is still being fetched writes it again when the piece
  arrives.
- Throwing away a directory row deletes its files but leaves the emptied folders.
- Opening a file inside the torrent and the Downloaded view rely on the engine,
  so they answer only while the torrent is in the session.
- Seeding is always on. `torrent_session::set_seeding_enabled` has no caller
  outside tests, and the `torrent.seed` settings key (`SEEDING_KEY`) is not read
  anywhere.

## Licence

MIT or Apache-2.0, at your option. Contributions are taken under the DCO; sign
off with `git commit -s`. The icons are covered separately, see
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
