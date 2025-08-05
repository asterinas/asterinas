<p align="center">
    <img src="docs/src/images/logo_en.svg" alt="asterinas-logo" width="620"><br>
    安全で高速、汎用的なOSカーネル。Rustで書かれ、Linuxと互換性があります<br/>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_x86.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_x86.yml/badge.svg?event=push" alt="Test x86-64" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_riscv.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_riscv.yml/badge.svg?event=push" alt="Test riscv64" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_loongarch.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_loongarch.yml/badge.svg?event=push" alt="Test loongarch64" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_x86_tdx.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_x86_tdx.yml/badge.svg" alt="Test Intel TDX" style="max-width: 100%;"></a>
    <a href="https://asterinas.github.io/benchmark/x86-64/"><img src="https://github.com/asterinas/asterinas/actions/workflows/benchmark_x86.yml/badge.svg" alt="Benchmark x86-64" style="max-width: 100%;"></a>
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

本番レベルのOSカーネルを目指す道のりは困難ですが、私たちはこの目標に向けて着実に前進しています。  
2024年を通じて、[年末レポート](https://asterinas.github.io/2025/01/20/asterinas-in-2024.html)に詳述されているように、Asterinasの成熟度を大幅に向上させました。  
そして2025年には、Asterinasをx86-64仮想マシン環境で本番運用可能なレベルに引き上げ、実際のユーザーを獲得することを主な目標としています。

## クイックスタート

Dockerがインストールされたx86-64 Linuxマシンを用意してください。
以下の3つの簡単なステップに従って、Asterinasを起動します。

1. 最新のソースコードをダウンロードします。

```bash
git clone https://github.com/asterinas/asterinas
```

2. 開発環境としてDockerコンテナを実行します。

```bash
docker run -it --privileged --network=host --device=/dev/kvm -v $(pwd)/asterinas:/root/asterinas asterinas/asterinas:0.16.0-20250802
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
