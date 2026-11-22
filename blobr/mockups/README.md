# Suite mockups

`suite.jdb` holds the jamsdesign mockups (Photosplash, Cinematica and Blobr main windows,
their component sheets, the Blobr logo), imported from the Figma file they were drawn in.
Open it in Blobr: `cargo run -p blobr -- blobr/mockups/suite.jdb`.

`suite.figma.json` is the Figma REST API's copy of that file. Re-import from it after
improving `jd-figma`, without touching Figma:

    cargo run -p jd-figma --bin figma2jdb -- blobr/mockups/suite.figma.json blobr/mockups/suite.jdb

From here on, design work happens in the `.jdb`; Figma's copy is history.
