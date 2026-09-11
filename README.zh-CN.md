# Nexa

> 面向本地文件、个人知识和日常工作的本地优先桌面助手。

[English](README.md) · [下载安装](https://github.com/MLGBJDLW/Nexa/releases/latest) · [文档索引](docs/README.md) · [贡献指南](CONTRIBUTING.md)

Nexa 将本地检索、基于证据的对话、文档处理和 Agent 工具整合在同一桌面工作台。
添加包含笔记、PDF、表格、演示文稿或图片的文件夹，即可检索资料、查看引用、整理证据，
并让助手协助完成具体任务。

索引、对话、集合和任务历史由本机管理。云端模型和外部服务接收当前操作需要的输入，
也可以使用本地模型。本地优先不代表所有功能都能离线运行：模型下载、云端推理、
网页检索、连接器及可选的手机公网连接需要网络。

## 主要能力

| 领域 | 当前能力 |
| --- | --- |
| 知识与检索 | 文件夹导入、增量索引、OCR、关键词与向量混合检索、来源筛选、Recall Mode，以及可追溯原文的知识图谱 |
| 对话 | 引用回答、集合与项目上下文、持久任务历史、流式工具状态、检查点、归档与恢复，以及数学公式和 Mermaid 图表 |
| 模型与 Agent | API 与本地模型连接、按端点识别能力、模型和推理档位选择、受支持的订阅 Agent、可复用 Skills 与已配置的 API 子代理 |
| 文件与 Office | 权限范围内的文件编辑、文档分析与生成、Office 产物校验与审阅、本地预览，以及单独配对的 Office.js 加载项 |
| 桌面与浏览器 | 对话关联终端、Browser Workspace、授权的 HTML 预览、结构化浏览器工具，以及带观察和审批边界的 Windows 电脑操作 |
| 语音与 Live | 在可编辑输入框内听写；按实际连接能力使用麦克风、摄像头或屏幕输入；保存文字观察记录和总结 |
| 手机连接 | 二维码配对、查看已有对话、选择模型、听写、文件预览、单次审批，以及通过加密局域网或公网通道使用 Live |
| 重复工作 | 集合、项目记忆、工作流模板、定时任务、MCP 连接器，以及由用户维护的 Skill 和主题文件 |

知识图谱是导航索引，关系本身不等于证据，使用结论前应查看支持它的原文。
工具是否可用取决于运行平台、连接配置、来源范围和权限；兼容某种 API 的端点并不自动
支持该提供方的全部功能。

## 安装与开始使用

在 [GitHub Releases](https://github.com/MLGBJDLW/Nexa/releases/latest) 下载与操作系统和 CPU 架构匹配的产物：

| 平台 | 发布包 |
| --- | --- |
| Windows | NSIS 安装程序（`.exe`） |
| macOS | 磁盘映像（`.dmg`）；应用配置要求 macOS 14 或更新版本 |
| Linux | AppImage |

具体可下载平台以该次 Release 的实际附件为准。原生电脑操作工具仅适用于 Windows；
浏览器、麦克风、媒体和 Office 集成也受宿主环境与已安装运行时限制。

1. 打开 Nexa，在设置中配置模型连接。API、本地模型和订阅连接的区别见
   [模型与提供方指南](docs/PROVIDERS_AND_MODELS.md)。
2. 在 Sources 中添加本地文件夹并等待索引。建议先使用一个较小的文件夹，
   添加大量资料前先检查排除规则。
3. 搜索一份已知文档，核对结果，再限定来源进行提问。可将重要证据保存到 Collection。
4. 处理文档时检查生成产物与校验结果；手机使用步骤见[手机连接指南](docs/remote-access.md)。

支持 Markdown、纯文本、日志、PDF、DOCX、XLSX、PPTX 和图片等输入。
OCR 和媒体处理需要对应模型及运行时，启用 Cargo feature 不等于安装了全部资源。

## 隐私与控制

- 本地文件、索引、对话和任务记录默认保存在电脑上。可选的 API 嵌入和云端工具会将
  所选输入发送至配置的服务。
- 来源排除和正则脱敏规则用于控制索引及发送给模型的内容，导入敏感目录前应检查设置。
- 文件、Shell、浏览器、电脑操作、终端和连接器具有各自的访问及审批策略。
- 配对手机可以读取对话和授权预览、启动 Agent 工作及响应单次审批；不再使用时可在
  桌面撤销设备权限。
- 公网隧道提供方承载远程流量，也可以使用加密局域网或自己配置的固定 HTTPS 地址。
- 项目本身不包含产品遥测流水线，外部服务有各自的数据处理规则。

## 从源码运行

使用 **Node.js 24**、[rust-toolchain.toml](rust-toolchain.toml) 指定的 Rust stable，
并安装[贡献指南](CONTRIBUTING.md)中列出的平台依赖。

```bash
git clone https://github.com/MLGBJDLW/Nexa.git
cd Nexa
npm ci
npm ci --prefix apps/desktop
cd apps/desktop
npm run tauri -- dev
```

仅开发前端时，在 `apps/desktop` 执行 `npm run dev`；它启动网页前端，不会启动原生桌面运行时。
桌面打包在同一目录执行 `npm run tauri -- build`。
资源准备、feature flags、测试和平台故障排查见[开发与验证](CONTRIBUTING.md#development)。

## 架构与仓库

Nexa 使用 Tauri 2、React 19.2、React Router 8.3、TypeScript、Rust 和 SQLite。精确依赖版本以清单和锁文件为准，
模型能力以共享目录与实际端点的发现结果为准。

```text
apps/desktop/               桌面与手机 React 前端
apps/desktop/src-tauri/     原生宿主、命令、浏览器、终端与远程桥接
crates/core/               Agent、检索、持久化、工具与媒体处理
crates/remote/             配对、HTTP/WebSocket 传输、TLS 与隧道
shared/                    提供方与能力目录
integrations/office-addin/  单独配对的 Office.js 客户端
tools/                     校验器与辅助运行时
scripts/                   仓库检查与发布工具
docs/                      用户和工程文档
testdata/                  测试样例
```

系统职责见[架构总览](docs/ARCHITECTURE.md)，各模块契约见[文档索引](docs/README.md)，
产品优先级见[路线图](docs/ROADMAP.md)。实验性或规划中的扩展格式在对应文档中单独标明。

## 语言

界面支持英语、简体中文、繁体中文、日语、韩语、西班牙语、法语、德语、葡萄牙语和俄语。

公开技术文档以英文为主，本中文 README 与英文版同步核心能力、使用步骤和限制。
界面翻译维护流程见[国际化指南](docs/I18N_GUIDELINES.md)。

## 贡献与许可

提交修改前请阅读[贡献指南](CONTRIBUTING.md)。问题可提交到
[GitHub Issues](https://github.com/MLGBJDLW/Nexa/issues)，请附版本、平台、复现步骤和已脱敏的诊断信息。

Nexa 使用 [MIT 许可](LICENSE)。第三方声明见
[Third-party notices](apps/desktop/THIRD_PARTY_NOTICES.md)。
