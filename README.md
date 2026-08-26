# CC Switch（Codex / OpenCode / WorkBuddy 版）

这是一个基于 Tauri 的桌面配置管理器。本版本对原项目进行了精简和定制，当前重点服务于 **Codex、OpenCode 和 WorkBuddy** 三个应用。

## 我做了什么

### 1. 精简项目范围

- 移除与当前需求无关的应用入口、配置页面和旧的管理逻辑。
- 保留 Codex、OpenCode 的提供商管理、配置切换和相关设置。
- 调整导入、编辑、设置和应用切换界面，使其围绕这几个应用工作。
- 清理无用配置和旧的前端代码，降低项目维护成本。

### 2. 加入 WorkBuddy 支持

- WorkBuddy 在应用切换器中与 Codex、OpenCode 处于同一级别。
- 支持配置 WorkBuddy 的接口地址、API Key、模型 ID 等提供商信息。
- 保存后配置写入本地数据库，重新打开编辑页面时会恢复已保存内容。
- 增加 WorkBuddy 专用图标，并支持在配置列表、切换器和代理页面中显示。
- 保留 WorkBuddy 原有模型配置，代理接管时只追加或更新 CC Switch 管理的路由；关闭接管后可以恢复原配置。

### 3. 增加 WorkBuddy 协议转换

WorkBuddy 自定义模型使用 Chat Completions 协议，而部分上游模型只接受 OpenAI Responses 协议。为此，项目新增了本地代理转换层：

```text
WorkBuddy Chat 请求
        ↓
CC Switch 本地代理
        ↓  Chat → Responses
上游 Responses 接口
        ↓  Responses → Chat
WorkBuddy Chat 响应
```

目前转换层覆盖：

- 普通文本消息、system/instructions、图片内容
- `max_tokens` / `max_completion_tokens`
- `reasoning_effort`
- tools、tool choice、函数调用和工具结果
- `response_format`
- 非流式响应和 SSE 流式响应
- usage 统计和常见上游错误信息

这样 WorkBuddy 无需原生支持 Responses 协议，也可以通过 CC Switch 使用 Responses 兼容的模型服务。

### 4. 完善代理接管流程

- 为 WorkBuddy 增加独立的配置读取、备份、接管、恢复和热切换逻辑。
- 代理启动时按当前选择的提供商生成托管路由。
- 代理关闭或切换回直连模式时，清理托管配置并尽量恢复用户原有配置。
- 对工具调用等容易导致请求失败的字段增加校验，避免错误请求直接转发到上游。

## 使用方式

1. 启动 CC Switch。
2. 在对应应用下新增或编辑提供商，填写接口地址、API Key 和模型 ID。
3. 选择提供商并保存。
4. 如果需要协议转换或统一转发，在代理页面开启对应应用的代理接管。
5. 重启或重新打开 WorkBuddy、Codex、OpenCode，使应用重新读取配置。

WorkBuddy 的上游接口需要能够处理 Responses 请求。CC Switch 负责协议转换，不会替上游服务增加模型权限；如果上游账号或 API Key 没有目标模型权限，仍会返回鉴权或模型不可用错误。

## 本地开发

环境要求：

- Node.js
- pnpm
- Rust 工具链

安装依赖并启动开发环境：

```bash
pnpm install
pnpm tauri dev
```

常用检查命令：

```bash
pnpm typecheck
pnpm test:unit
```

构建桌面安装包：

```bash
pnpm tauri build
```

Windows 安装包会生成在 `src-tauri/target/release/bundle/` 对应的安装包目录中。

## 项目结构

- `src/`：React 前端界面和提供商表单
- `src-tauri/src/`：Rust 后端、配置读写、本地数据库和代理服务
- `src-tauri/src/proxy/providers/workbuddy.rs`：WorkBuddy Chat/Responses 协议转换
- `src-tauri/src/workbuddy_config.rs`：WorkBuddy `models.json` 配置处理
- `tests/`：前端和集成测试

## 数据与安全

- 提供商配置保存在本机应用数据目录的本地数据库中。
- 代理服务默认运行在本机，不会把配置提交到仓库。
- 请不要把真实 API Key、OAuth Token 或私钥写入源码、测试文件或提交记录。

## 许可证

本项目沿用原项目的 MIT 许可证。详见 [LICENSE](LICENSE)。
