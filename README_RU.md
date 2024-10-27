<p align="center">
    <img src="docs/src/images/logo_en.svg" alt="asterinas-logo" width="620"><br>
    Безопасное, быстрое и универсальное ядро ОС, написанное на Rust и совместимое с Linux<br/>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_osdk.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_osdk.yml/badge.svg?event=push" alt="Test OSDK" style="max-width: 100%;"></a>
    <a href="https://github.com/asterinas/asterinas/actions/workflows/test_asterinas.yml"><img src="https://github.com/asterinas/asterinas/actions/workflows/test_asterinas.yml/badge.svg?event=push" alt="Test Asterinas" style="max-width: 100%;"></a>
    <a href="https://asterinas.github.io/benchmark/"><img src="https://github.com/asterinas/asterinas/actions/workflows/benchmark_asterinas.yml/badge.svg" alt="Benchmark Asterinas" style="max-width: 100%;"></a>
    <br/>
</p>

[English](README.md) | [中文版](README_CN.md) | [日本語](README_JP.md) | Русский

## Представляем Asterinas

Asterinas - это _безопасное_, _быстрое_ и _универсальное_ ядро ОС,
предоставляющее _совместимый с Linux_ ABI.
Оно может служить полноценной заменой Linux,
одновременно повышая _безопасность памяти_ и _удобство для разработчиков_.

* Asterinas уделяет приоритетное внимание безопасности памяти,
используя Rust в качестве единственного языка программирования
и ограничивая использование _небезопасного Rust_
четко определенной и минимальной доверенной вычислительной базой (TCB).
Этот инновационный подход,
известный как [архитектура framekernel](https://asterinas.github.io/book/kernel/the-framekernel-architecture.html),
делает Asterinas более безопасным и надежным вариантом ядра.

* Asterinas превосходит Linux с точки зрения удобства для разработчиков.
Он позволяет разработчикам ядра
(1) использовать более продуктивный язык программирования Rust,
(2) применять специально разработанный инструментарий под названием [OSDK](https://asterinas.github.io/book/osdk/guide/index.html) для оптимизации рабочих процессов,
и (3) выбирать между выпуском своих модулей ядра с открытым исходным кодом
или сохранением их в качестве проприетарных,
благодаря гибкости, предоставляемой [MPL](#Лицензия).

Хотя путь к ядру ОС производственного уровня может быть сложным,
мы неуклонно продвигаемся к нашей цели.
В настоящее время Asterinas поддерживает только виртуальные машины x86-64.
Однако [наша цель на 2024 год](https://asterinas.github.io/book/kernel/roadmap.html) -
сделать Asterinas готовым к производству на виртуальных машинах x86-64.

## Начало работы

Подготовьте машину с Linux x86-64 с установленным Docker.
Выполните три простых шага ниже, чтобы запустить Asterinas.

1. Загрузите последний исходный код.

```bash
git clone https://github.com/asterinas/asterinas
```

2. Запустите контейнер Docker в качестве среды разработки.

```bash
docker run -it --privileged --network=host --device=/dev/kvm -v $(pwd)/asterinas:/root/asterinas asterinas/asterinas:0.9.4
```

3. Внутри контейнера перейдите в папку проекта, чтобы собрать и запустить Asterinas.

```bash
make build
make run
```

Если все прошло успешно, Asterinas теперь работает внутри виртуальной машины.

## Книга

Смотрите [Книгу Asterinas](https://asterinas.github.io/book/), чтобы узнать больше о проекте.

## Лицензия

Исходный код и документация Asterinas в основном используют 
[Mozilla Public License (MPL), Version 2.0](https://github.com/asterinas/asterinas/blob/main/LICENSE-MPL).
Отдельные компоненты находятся под более разрешительными лицензиями,
подробности [здесь](https://github.com/asterinas/asterinas/blob/main/.licenserc.yaml). Обоснование выбора MPL см. [здесь](https://asterinas.github.io/book/index.html#licensing).
