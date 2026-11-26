<div align="center">

# KoraSSH

### 你管理的每一台服务器，都在同一个窗口里。仪表盘、终端、SFTP、命令片段。

<img src="assets/gui-preview.png" alt="KoraSSH 桌面端" width="900">

[![Downloads](https://img.shields.io/github/downloads/AnonyJcy/korassh/total?label=total%20installs&color=2ea44f)](https://github.com/AnonyJcy/korassh/releases)
[![Latest release](https://img.shields.io/github/v/release/AnonyJcy/korassh?label=latest)](https://github.com/AnonyJcy/korassh/releases/latest)
[![Stars](https://img.shields.io/github/stars/AnonyJcy/korassh?style=flat)](https://github.com/AnonyJcy/korassh/stargazers)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Build](https://img.shields.io/github/actions/workflow/status/AnonyJcy/korassh/ci.yml?branch=main)](https://github.com/AnonyJcy/korassh/actions)

**[安装](#安装)** •
**[它能做什么](#它能做什么)** •
**[SSH 密钥配置](#ssh-密钥配置)** •
**[终端版](#终端版)** •
**[常见问题](#常见问题)** •
**[参与贡献](#参与贡献)**

</div>

---

## 安装

macOS 和 Linux 上一条命令：

```bash
curl -fsSL https://raw.githubusercontent.com/AnonyJcy/korassh/main/install.sh | sh
```

脚本会识别系统和架构，装上最新的桌面版：macOS 放进「应用程序」，Linux 进应用菜单。想要终端版，或者两个都要？`curl … | sh -s -- --tui`（或 `--both`）。更喜欢自己点？到 [**Releases**](https://github.com/AnonyJcy/korassh/releases/latest) 取对应平台的文件。

| 平台 | 文件 |
|------|------|
| macOS Apple Silicon | `KoraSSH-aarch64-apple-darwin.dmg` |
| macOS Intel | `KoraSSH-x86_64-apple-darwin.dmg` |
| Linux x86_64 | `KoraSSH-x86_64.AppImage` / `.deb` / `.rpm` |
| Windows x86_64 | `KoraSSH-x86_64-setup.exe` |

没有账号，没有登录界面，没有遥测。第一次打开是一个空的仪表盘；如果本机有 `~/.ssh/config`，它会直接读进来，经跳板机（`ProxyJump`）访问的主机也算在内。

从 OmnySSH（中文维护版）过来？主机、命令片段和设置会在 KoraSSH 第一次启动时自动带过去，旧目录原样保留。

---

## 它能做什么

主机只添加一次。之后它就待在仪表盘上，一张卡片，带实时的 CPU、内存、磁盘、运行时长、发行版，有吃 CPU 的进程，还有一个说明它在跑什么的服务标记。点一下「终端」就进到真实的 PTY，点一下「SFTP」就打开双栏文件管理器。十台服务器放在同一屏，自己刷新。

### 实时仪表盘

每台主机一张卡片：CPU、内存、磁盘的占用条，运行时长，系统版本，最占 CPU 的进程，Docker 的标记会写明有几个容器在跑。占用条先变黄，再变红，一台不舒服的服务器隔着房间也看得出来。按标签分组、排序；桌面版还能把卡片拖成自己的顺序。

### 终端

完整的 PTY 会话，开成标签页。想开几台就开几台，从侧栏切换，回仪表盘干活的时候它们照样在跑。左右或上下分屏，两台服务器并排看；断线了点一下重连；输出里的链接可以直接点开。

### 双栏 SFTP

左边本地，右边远程。勾上要传的文件和目录，拉过去就行；也可以直接从一边拖到另一边，或者从文件管理器拖到服务器上。看着进度条走完，一次选一批，每个文件最后改于什么时候一目了然。远程的文本文件可以在线打开改完再保存回去（`Ctrl+S`），权限列旁边就是 `chmod`。反正也没人记得 `scp -r` 的语法。

### 命令片段

把每周都要敲一遍的命令存起来。选一条，勾上要发往的主机，它会同时在所有主机上运行。片段可以带参数：`sudo systemctl restart {{service}}` 会先问你服务名。终端底下那条可收起的快捷命令栏，一点就把命令发进当前的会话。

### 端口转发与代理

本地转发（`-L`）、远程转发（`-R`）、SOCKS5 代理（`-D`）在主机编辑器里各占一行，卡片上一键开关。主机前面有 HTTP 或 SOCKS5 代理也能配；经跳板机（`ProxyJump`）访问的主机从 `~/.ssh/config` 读进来。

### 密码加密与同步

主机和代理的密码默认以 AES-256-GCM 加密后落盘，磁盘上没有明文。想再加一道锁，就设一个主密码（Argon2id），启动后输入一次。主机、片段和设置可以导出成端到端加密的备份文件带走，也可以放进 WebDAV 或 Git 仓库，在几台电脑之间同步。离开本机的只有密文。

### 全局搜索

⌘K（Windows 和 Linux 上是 Ctrl+K），然后直接打字。你有的每一台主机，加上已经打开的每一个会话。回车就落到所选主机的终端里，或者回到刚才离开的会话。

### 主播模式

把屏幕上所有真实 IP 换成假的。录 demo 或者共享屏幕，不会泄露客户的基础设施。

### 明暗主题，字体随你

两套主题都在，侧栏里切换。界面字体和终端字体各自可换；终端另有 Dracula、Nord、Tokyo Night、Solarized 等七套配色。

### 轻量

终端版 `kora` 是一个 5 MB 左右的静态二进制。桌面版的 dmg、安装程序、deb 和 rpm 在 6 到 8 MB 之间。

---

## SSH 密钥配置

新 VPS 还在用密码登录，是那种你一直想着要改、却从来没改的事。KoraSSH 一次点击搞定。

挑一台你自己添加的、还没配密钥的主机，点**自动配置 SSH 密钥**，它会生成一把 Ed25519 密钥，把公钥追加到 `authorized_keys`，然后把这台主机切换成密钥登录。点下去会先让你确认一次。确认之后，它会用新密钥重新连一次，证明密钥确实能用，然后才关掉密码登录；流程一旦开始就会走完。

碰 `sshd_config` 之前，它会在服务器上留一份备份。任何一步失败，就恢复备份，访问方式原样不动。私钥从不离开你的机器，除了你选的那台服务器，什么也不会发往别处。密码登录关掉以后，卡片上还留着「恢复密码登录」这条路。

代码在 [`crates/korassh-core/src/ssh/key_setup.rs`](crates/korassh-core/src/ssh/key_setup.rs)。把它指向生产环境之前，先读一遍。开源的意义就在这里。

带口令的密钥也能用。密钥是加密的，KoraSSH 会问一次口令，退出之前留在内存里，从不写进磁盘。在 macOS 和 Linux 上，已经加载进 `ssh-agent` 的密钥直接拿来用，不会再问。主机的**密码**栏填的是服务器登录密码，不是密钥口令。

---

## 终端版

KoraSSH 是从终端里长出来的，TUI 还在，还在维护，也照样发版。

![Demo](assets/demo.gif)

底下是同一套引擎：仓库是一个 cargo workspace，`crates/korassh-core` 放逻辑，各个前端建在它上面。仪表盘、SFTP、命令片段、多会话标签（可以分屏）、模糊搜索，四套主题（`default`、`dracula`、`nord`、`gruvbox`），按键可以在 `config.toml` 里改。

```bash
# Nix（仓库自带 flake.nix）
nix run github:AnonyJcy/korassh

# cargo，从源码装
cargo install --git https://github.com/AnonyJcy/korassh korassh
```

然后运行 `kora`。按 `a` 添加主机，`/` 搜索，`g` 按标签分组，`?` 看帮助，`K` 给选中的主机配置密钥。

Linux、macOS、Windows 和 Termux 的预编译二进制在 [Releases](https://github.com/AnonyJcy/korassh/releases) 页的 `kora-*` 文件里。配置放在 `~/.config/korassh/`（Linux）、`~/Library/Application Support/korassh/`（macOS）、`%APPDATA%\korassh\`（Windows）。`~/.ssh/config` 只在启动时读取，从不写入。

完整的选项、按键和配置示例：`man kora`。

---

## 常见问题

**macOS：局域网里的主机连不上，报 "No route to host"。** macOS 只在得到允许之后才让应用访问局域网设备。到「系统设置 → 隐私与安全性 → 本地网络」把 KoraSSH 打开（已经开着就关了再开），然后重新打开应用。终端版 `kora` 跑在哪个终端应用里，就是那个应用需要这项权限；系统自带的「终端」已经有了。

**群晖：输完密码，终端立刻以 "Permission denied, please try again." 结束。** DSM 只给 administrators 群组的成员开 shell，而且要在「控制面板 → 终端机和 SNMP → 启动 SSH 功能」打开的情况下。其他账号仍然可以用 SFTP。

**Windows：便携版 `KoraSSH-x86_64.zip` 双击没有窗口。** 便携版不带 WebView2 运行时。装 `KoraSSH-x86_64-setup.exe`，安装程序会把它补上。

---

## 参与贡献

欢迎提 PR。[CONTRIBUTING.md](CONTRIBUTING.md) 里有环境搭建、约定和检查清单。想做大改动，先开个 issue，免得我们俩各做一遍。

工作区结构：

```
crates/korassh-core   引擎，不含界面
crates/korassh        终端版（二进制：kora）
crates/korassh-gui    Tauri 桌面版
```

## 协议

Apache 2.0，见 [LICENSE](LICENSE)。

KoraSSH 最初衍生自 [OmnySSH](https://github.com/timhartmann7/omnyssh)（Apache-2.0），现已独立演进维护。原始提交历史与版权声明完整保留，另见 [NOTICE](NOTICE)。

<div align="center">

### ⭐ 如果 KoraSSH 替你省下了一个终端标签，点个 star

[报告问题](https://github.com/AnonyJcy/korassh/issues) •
[功能建议](https://github.com/AnonyJcy/korassh/issues) •
[Releases](https://github.com/AnonyJcy/korassh/releases)

</div>
