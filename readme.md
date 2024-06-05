<div align="center">
  <img  width="256" height="256" src="/docs/src/assets/logo-without-bg.png" />
  <h1>账 Zhang</h1>
  <p>a plain text double-accounting tool.</p>
</div>

![GitHub Workflow Status (with event)](https://img.shields.io/github/actions/workflow/status/zhang-accounting/zhang/build-latest.yml)
[![](https://codecov.io/gh/zhang-accounting/zhang/branch/main/graph/badge.svg?token=AVM0HNGF91)](https://codecov.io/gh/zhang-accounting/zhang)
![Crates.io (recent)](https://img.shields.io/crates/dr/zhang)
![Docker Pulls](https://img.shields.io/docker/pulls/kilerd/zhang)
[![](https://img.shields.io/docsrs/zhang)](docs.rs/zhang)
![](https://img.shields.io/crates/l/zhang)

- Online Playground: [Online Playground](https://zhang-playground.zeabur.app/)
- Online Demo: [Online Demo](https://zhang-accounting.zeabur.app/)
- Documentation: [Documentation](https://zhang-accounting.kilerd.me/)

[![Discord Banner 2](https://discord.com/api/guilds/1217736070045896704/widget.png?style=banner2)](https://discord.gg/EGjwhnV267)
[![testflight icon](/assets/TestFlight_Light.svg)](https://testflight.apple.com/join/3pm50he2)

## Features

- **Independent Direcitve**: all directives in zhang are independent, you can write them in any file with any order.
- **More Precise Control**: features, like commodity decimal precision and datetime supported for directive, provide
  more control
- **Document Management Enhancement**: zhang has a good document management feature to allow you collect and control
  document easiler and effective, like receipts.

### Compatibility with beancount

beancount and zhang are both text based accounting tools, and they are some familiar.

But zhang deprecates some directives, like `note`, `pad` and `push_tag`, and aslo evolves some direcitves,
like `balance`. So your beancount file may not be compatible with zhang, we will provide a tool to convert beancount
format to zhang format, and vice versa.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=zhang-accounting/zhang&type=Date)](https://star-history.com/#zhang-accounting/zhang&Date)