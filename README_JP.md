<p align="center">
    <img src="docs/src/images/logo_en.svg" alt="asterinas-logo" width="620"><br>
    安全で高速、汎用的なOSカーネル。Rustで書かれ、Linuxと互換性があります<br/>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_osdk.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_osdk.yml/badge.svg?event=push" alt="Test OSDK" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_asterinas.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_asterinas.yml/badge.svg?event=push" alt="Test Asterinas" style="max-width: 100%;"></a>
    <a href="https://asterinas.github.io/benchmark/"><img src="https://github.com/asterinas/asterinas/actions/workflows/benchmark_asterinas.yml/badge.svg" alt="Benchmark Asterinas" style="max-width: 100%;"></a>
    <br/>
</p>

[English](README.md) | [中文版](README_CN.md) | 日本語

## Asterinasの紹介

Asterinasは、安全で高速、汎用的なOSカーネルです。
Linux互換のABIを提供し、Linuxの代替としてシームレスに動作します。
また、メモリの安全性と開発者の利便性を向上させます。

* Asterinasは、Rustを唯一のプログラミング言語として使用し、
  _unsafe Rust_ の使用を明確に定義された最小限の信頼できるコンピューティングベース（TCB）に制限することで、
  メモリの安全性を最優先します。
  この革新的なアプローチは、[フレームカーネルアーキテクチャ](https://asterinas.github.io/book/kernel/the-framekernel-architecture.html)として知られ、
  Asterinasをより安全で信頼性の高いカーネルオプションとして確立します。

* Asterinasは、開発者の利便性においてもLinuxを上回ります。
  カーネル開発者は、より生産性の高いRustプログラミング言語を利用し、
  専用のツールキットである[OSDK](https://asterinas.github.io/book/osdk/guide/index.html)を活用してワークフローを簡素化し、
  [MPL](#License)の柔軟性を活かして、カーネルモジュールをオープンソースとして公開するか、
  プロプライエタリとして保持するかを選択できます。

生産グレードのOSカーネルへの道のりは困難を伴いますが、
私たちは目標に向かって着実に進んでいます。
現在、Asterinasはx86-64仮想マシンのみをサポートしています。
しかし、[2024年の目標](https://asterinas.github.io/book/kernel/roadmap.html)は、
x86-64仮想マシンでAsterinasを生産準備完了にすることです。

## クイックスタート

Dockerがインストールされたx86-64 Linuxマシンを用意してください。
以下の3つの簡単なステップに従って、Asterinasを起動します。

1. 最新のソースコードをダウンロードします。

```bash
git clone https://github.com/asterinas/asterinas
```

2. 開発環境としてDockerコンテナを実行します。

```bash
docker run -it --privileged --network=host --device=/dev/kvm -v $(pwd)/asterinas:/root/asterinas asterinas/asterinas:0.9.4
```

3. コンテナ内でプロジェクトフォルダに移動し、Asterinasをビルドして実行します。

```bash
make build
make run
```

すべてが順調に進めば、Asterinasは仮想マシン内で実行されます。

## ドキュメント

プロジェクトの詳細については、[The Asterinas Book](https://asterinas.github.io/book/)をご覧ください。

## ライセンス

Asterinasのソースコードとドキュメントは主に
[Mozilla Public License (MPL), Version 2.0](https://github.com/asterinas/asterinas/blob/main/LICENSE-MPL)を使用しています。
一部のコンポーネントは、より寛容なライセンスの下で提供されています。
詳細は[こちら](https://github.com/asterinas/asterinas/blob/main/.licenserc.yaml)をご覧ください。
MPLを選択した理由については、[こちら](https://asterinas.github.io/book/index.html#licensing)をご覧ください。
