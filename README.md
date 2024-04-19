# dirplayer-rs

DirPlayer is a Shockwave Player emulator written in Rust that aims to make playing old browser games possible on modern browsers.

## Installing dependencies
```bash
npm install
```

## Building Rust VM

```bash
cd vm-rust
wasm-pack build --target web
```

## Running locally

```bash
npm run start
```

## Acknowledgements

This project would have not been possible without the extensive work of the Shockwave reverse engineering community.

A lot of code has been reproduced from the following projects:

https://github.com/Earthquake-Project/Format-Documentation/

https://github.com/Brian151/OpenShockwave/

https://gist.github.com/MrBrax/1f3ae06c9320863f1d7b79b988c03e60

https://www.scummvm.org/
