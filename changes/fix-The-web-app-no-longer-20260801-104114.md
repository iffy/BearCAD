The web app no longer fails to start with a LinkError after a deploy: every asset URL carries its build, so a browser can't pair a cached script with a fresh wasm module.
