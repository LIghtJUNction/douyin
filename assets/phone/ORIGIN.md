# Java bridge

`java-bridge.js` is the unmodified bundle distributed in `frida-tools` 14.13.1,
file `frida_tools/bridges/java.js`, from the official PyPI distribution.

- SHA-256: `b3e38898cfb44f0ceab6d6b6e03ac94620bcff3c7dfef73b0953bbc95ac0e4ee`
- Upstream: https://github.com/frida-packages/frida-java-bridge
- License: LGPL-2.0 WITH WxWindows-exception-3.1; see COPYING and COPYING.LIB.

`agent.js` is this project's own integration code. The bridge obtains current
request parameters and uses the installed application's GET provider; it does
not return Cookies, security keys, or signed headers to the computer.

`server-assets.json` pins the official Frida 17.9.5 Android release assets and
their GitHub-published SHA-256 digests, matching frida-sys 0.17.2's core version.
