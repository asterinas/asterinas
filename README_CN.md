<p align="center">
    <img src="docs/src/images/logo_cn.svg" alt="asterinas-logo" width="620"><br>
    一个安全、快速、通用的操作系统内核，使用Rust编写，并与Linux兼容<br/>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/osdk_test.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/osdk_test.yml/badge.svg?event=push" alt="OSDK Test" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/kernel_test.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/kernel_test.yml/badge.svg?event=push" alt="Kernel Test" style="max-width: 100%;"></a>
    <br/>
</p>

[English](README.md) | 中文版

## 初见星绽

星绽（英文名：Asterinas）是一个*安全*、*快速*、*通用*的操作系统内核。
它提供于Linux相同的ABI，可无缝运行Linux应用，
但比Linux更加*内存安全*和*开发者友好*。

* 星绽在内存安全性方面远胜Linux。
它使用Rust作为唯一的编程语言，
并将*unsafe Rust*的使用限制在一个明确定义且最小的可信计算基础（TCB）上。
这种新颖的方法，
被称为[框内核架构](https://asterinas.github.io/book/kernel/the-framekernel-architecture.html)，
使星绽成为一个更安全、更可靠的内核选择。

* 星绽在开发者友好性方面优于Linux。
它赋能内核开发者们
（1）使用生产力更高的Rust编程语言，
（2）利用一个专为内核开发者设计的工具包（称为[OSDK](https://asterinas.github.io/book/osdk/guide/index.html)）来简化他们的工作流程，
（3）享受[MPL](#License)所带来的灵活性，
可自由选择开源或闭源他们为星绽所开发的内核模块或驱动。

虽然通往生产级操作系统内核的路上注定充满艰险，
但我们坚信正朝着正确的方向迈进。
当前，星绽仅支持x86-64虚拟机；
[我们2024年的目标](https://asterinas.github.io/book/kernel/roadmap.html)是
使星绽在x86-64虚拟机环境中达到生产可用。

## 快速上手

准备一台安装了Docker的、x86-64架构的Linux机器。
按照以下三个简单的步骤来构建和启动星绽。

1. 下载最新的源代码。

```bash
git clone https://github.com/asterinas/asterinas
```

2. 运行一个作为开发环境的Docker容器。

```bash
docker run -it --privileged --network=host --device=/dev/kvm -v ./asterinas:/root/asterinas asterinas/asterinas:0.4.2
```

3. 在容器内，进入项目文件夹构建并运行星绽。

```bash
make build
make run
```

如果一切顺利，星绽现在应该在一个虚拟机内运行起来了。

## 技术文档

查看[The Asterinas Book](https://asterinas.github.io/book/)
以了解更多关于本项目的信息。

## 开源许可

星绽的源代码和文档主要使用
[Mozilla公共许可证（MPL），版本2.0](https://github.com/asterinas/asterinas/blob/main/LICENSE-MPL)，
部分组件在更宽松的许可证下发布，
详见[这里](https://github.com/asterinas/asterinas/blob/main/.licenserc.yaml)。
关于选择MPL的原因，请见[这里](https://asterinas.github.io/book/index.html#licensing)。
