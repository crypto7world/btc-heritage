<!-- markdownlint-disable MD033 MD041 -->
<div id="top"></div>

<!-- PROJECT SHIELDS -->
<!--
*** I'm using markdown "reference style" links for readability.
*** Reference links are enclosed in brackets [ ] instead of parentheses ( ).
*** See the bottom of this document for the declaration of the reference variables
*** for contributors-url, forks-url, etc. This is an optional, concise syntax you may use.
*** https://www.markdownguide.org/basic-syntax/#reference-style-links
-->

[![Contributors][contributors-shield]][contributors-url]
[![Forks][forks-shield]][forks-url]
[![Stargazers][stars-shield]][stars-url]
[![Issues][issues-shield]][issues-url]
[![MIT License][license-shield]][license-url]

<!-- PROJECT LOGO -->
<br />
<div align="center">
  <a href="https://github.com/crypto7world/btc-heritage">
    <img src="images/logo.png" alt="Logo" width="160" height="160">
  </a>

  <h3 align="center">BTC Heritage</h3>

  <p align="center">
    Rust crates implementing primitives for a Bitcoin Taproot wallet managing on-chain inheritance of coins
    <br />
    <a href="https://btcherit.com"><strong>Explore the Heritage wallet service »</strong></a>
    <br />
    <br />
    <a href="https://github.com/crypto7world/btc-heritage/issues">Report Bug</a>
    ·
    <a href="https://github.com/crypto7world/btc-heritage/issues">Request Feature</a>
  </p>
</div>

<!-- TABLE OF CONTENTS -->
<details>
  <summary>Table of Contents</summary>
  <ol>
    <li><a href="#about-the-project">About The Project</a></li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#roadmap">Roadmap</a></li>
    <li><a href="#built-with">Built With</a></li>
    <li><a href="#minimum-supported-rust-version-msrv">Minimum Supported Rust Version (MSRV)</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>

<!-- ABOUT THE PROJECT -->

## About The Project

The repository hosts 3 crates supporting the Bitcoin Heritage wallet: a Taproot Bitcoin wallet developped in _Rust_ with built-in, on-chain protections against loosing your coins and inheritance.

The basic principle is a dead-man switch: should you become unable to spend your coins for whatever reasons, alternative spending paths (i.e. TapScripts) will eventualy open, allowing other private keys to spend your coins, following a schedule you configure **enforced by the Bitcoin blockchain**. On the other hand, if you are able to spend your coin you can regularly "reset" this schedule simply by moving your coins to a new address of your wallet.

The **Heritage wallet** offers a trustless solution to protect your coins mainly in 2 situations:

1. You loose access to your wallet for some reason (backup issues, passphrase forgotten, ...)
2. You die.

In both cases, using the **Heritage wallet**, your coins will be recoverable after some time, either by yourself in situation 1 or your next of kin in situation 2.

Usually, protecting yourself against those situations require one or more trusted third-party with whom you share control of your coins to various degrees. The **Heritage wallet** offers a solution without such compromise: you retain exclusive control of your coins.

The 3 crates roles are:

- The `btc-heritage` library provides the fundations of the on-chain management, i.e. generating and tracking addresses, synchronizing with the blockchain, creating PSBT, basicaly everything not related to the private keys of the wallet;
- The `btc-heritage-wallet` library builds upon `btc-heritage` to add the features required to build a complete wallet software, most notably private keys management and signing capabilities;
- The `heritage-service-api-client` library exposes serializable types to communicate with the Heritage service API (see [btcherit.com][heritage-wallet-service]).

## Usage

Visit [btcherit.com][heritage-wallet-service], the online service built upon the `btc-heritage` library, to start using the **Heritage wallet** and learn more.

You will need to install the [heritage-cli] to manage your wallet's private keys.

The project is in a beta-phase: I use it to manage my own BTC confidently and so can you.

While I hope the [btcherit.com][heritage-wallet-service] service will help me pay my bills, I do not wish to lock users in and it is very important for me to allow people to manage their coins on their own if they wish to. So if you do not wish to use an online service, you can use only the [heritage-cli] with your own Bitcoin or Electrum node for synchronization!

<p align="right">(<a href="#top">back to top</a>)</p>

<!-- STABILITY AND VERSIONING -->

## Stability and versioning

Commits between releases SHOULD NOT be considered stable. You should only use tagged releases.

All the software provided is in working order and can be safely used to manage Bitcoin's holdings (I do since February 2024).

We are using [Semantic Versioning](https://github.com/semver/semver) (MAJOR.MINOR.PATCH).

Everything is still in the initial-development stage (version 0.x.x). While you can expect every new version to be in working order, you _SHOULD NOT_ expect the APIs to be stable. That being said, new features and breaking changes will only happen on MINOR version increment, not on PATCH version increment.

<!-- ROADMAP -->

## Roadmap

The roadmap is accurate regarding my immediate goals for the project.

- [x] Add on-chain/public TapRoot wallet capabilities to manage an Heritage wallet (wallet with an inheritance configuration you choose)
- [x] Add off-chain/private TapRoot wallet capabilities, i.e. private key management, the ability to sign transactions
- [x] Create a new CLI Heritage wallet tool
- [x] Add support for hardware wallets:
  - [x] Ledger ([ledger.com](https://www.ledger.com/))
  - [ ] ~~Trezor~~ (unfortunately Taproot script paths not supported currently) ([trezor.io](https://trezor.io/))
- [x] Capability for the wallet library to use a custom Bitcoin Core or Electrum node instead of the service
- [ ] Create a GUI Heritage wallet tool (see [btcherit.com][heritage-wallet-service], the online service in the time being)
- [ ] Add support for MultiSig N-of-M Heir to allow kind-of inheritance sharing
- [ ] Add lightning support
- [ ] Eliminate dependency to BDK

Also consult the [open issues](https://github.com/crypto7world/btc-heritage/issues) for other proposed features and known issues.

<p align="right">(<a href="#top">back to top</a>)</p>

## Built With

[![Rust][rust-shield]][rust-url]

And based upon 3 Rust projects without which I would not have gotten that far:

- [`bdk`]
- [`rust-miniscript`]
- [`rust-bitcoin`]

Thanks.

<p align="right">(<a href="#top">back to top</a>)</p>

<!-- MSRV -->

## Minimum Supported Rust Version (MSRV)

This library compile with Rust 1.79.0.

While I will always remain on stable Rust (i.e. _NOT_ depend on nightly), I do not plan on being conservative on the MSRV. If at some point a mildly interesting feature pops in a new Rust version, I will happily bump up the MSRV.

<p align="right">(<a href="#top">back to top</a>)</p>

<!-- CONTRIBUTING -->

## Contributing

Contributions are what make the open source community such an amazing place to learn, inspire, and create. Any contributions you make are **greatly appreciated**.

If you have a suggestion that would make this better, please fork the repo and create a pull request. You can also simply open an issue with the tag "enhancement".
Don't forget to give the project a star! Thanks again!

1. Fork the Project
2. Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3. Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the Branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

Your contribution will be licensed under the MIT license of this repository.

<p align="right">(<a href="#top">back to top</a>)</p>

<!-- LICENSE -->

## License

Distributed under the MIT License. See [`LICENSE`][license-url] for more information.

<p align="right">(<a href="#top">back to top</a>)</p>

<!-- CONTACT -->

## Contact

John Galt - [@Crypto7W](https://twitter.com/Crypto7W) - <john@crypto7.world>

Though my real name is Jérémie Rodon ([LinkedIn][jr-linkedin-url], [GitHub][jr-github-url]), I operate this project under the pseudonym John Galt in reference to the character of _Ayn Rand_ novel [**Atlas Shrugged**](https://www.amazon.com/Atlas-Shrugged-Ayn-Rand-ebook/dp/B003V8B5XO) (and, yes, I obviously embrace John Galt philosophy).

Project Link: [https://github.com/crypto7world/btc-heritage][repo-url]

<p align="right">(<a href="#top">back to top</a>)</p>

<!-- ACKNOWLEDGMENTS -->

## Acknowledgments

- [`rust-miniscript`]
- [`rust-bitcoin`]
- [`bdk`]
- [Best Readme Template](https://github.com/othneildrew/Best-README-Template)
- [Img Shields](https://shields.io)

<p align="right">(<a href="#top">back to top</a>)</p>

<!-- MARKDOWN LINKS & IMAGES -->
<!-- https://www.markdownguide.org/basic-syntax/#reference-style-links -->

[heritage-wallet-service]: https://btcherit.com
[heritage-cli]: https://github.com/crypto7world/heritage-cli
[repo-url]: https://github.com/crypto7world/btc-heritage
[contributors-shield]: https://img.shields.io/github/contributors/crypto7world/btc-heritage.svg?style=for-the-badge
[contributors-url]: https://github.com/crypto7world/btc-heritage/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/crypto7world/btc-heritage.svg?style=for-the-badge
[forks-url]: https://github.com/crypto7world/btc-heritage/network/members
[stars-shield]: https://img.shields.io/github/stars/crypto7world/btc-heritage.svg?style=for-the-badge
[stars-url]: https://github.com/crypto7world/btc-heritage/stargazers
[issues-shield]: https://img.shields.io/github/issues/crypto7world/btc-heritage.svg?style=for-the-badge
[issues-url]: https://github.com/crypto7world/btc-heritage/issues
[license-shield]: https://img.shields.io/github/license/crypto7world/btc-heritage.svg?style=for-the-badge
[license-url]: https://github.com/crypto7world/btc-heritage/blob/master/LICENSE
[jr-linkedin-url]: https://linkedin.com/in/JeremieRodon
[jr-github-url]: https://github.com/JeremieRodon
[rust-shield]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white
[rust-url]: https://https://www.rust-lang.org/
[`rust-miniscript`]: https://github.com/rust-bitcoin/rust-miniscript
[`rust-bitcoin`]: https://github.com/rust-bitcoin/rust-bitcoin
[`bdk`]: https://github.com/bitcoindevkit/bdk
