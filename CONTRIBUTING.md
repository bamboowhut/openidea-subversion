# Contributing

Open an issue for reproducible bugs and proposed changes. Include the OpenIDEA and plugin versions, the operating system, the SVN client version, and a small working-copy layout or reproduction when possible. Redact credentials, private URLs, and source code from logs and examples.

## Development

1. Fork the repository and create a branch for your change.
2. Follow the [build instructions](README.md#build-and-test) and run the tests against a local SVN fixture.
3. For UI or workflow changes, try the affected steps in the [manual verification guide](docs/manual-testing.md).
4. Open a pull request with the behavior change and the checks you ran.

Keep manifest labels and application source text in English. Add Simplified Chinese translations to `locales/zh-CN.json` for native labels and Rust messages, and to `web/zh-CN.js` for tool-window text. Keep command IDs, SVN flags, option values, and user-entered text independent of the display language.

## Releases

Update `plugin.toml`, `Cargo.toml`, `Cargo.lock`, and [CHANGELOG.md](CHANGELOG.md) together. Build and verify the package from the release revision. Attach the checked `.oiplugin` to the corresponding GitHub Release. For marketplace distribution, sign it using a publisher key stored outside the checkout, then submit the package URL, version, and SHA-256 to the community registry. [Registry instructions](https://github.com/l0ng-ai/openidea-plugins#readme)
