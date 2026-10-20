# Tinnitus Music

可复现的音乐生成与耳鸣聆听研究工具。Rust 引擎负责音频处理和验证，
Tauri / React 桌面端负责创作、播放、参与者配置与日志。

**流程：生成或导入音乐 → 验证音频 → 按当前配置处理 → 主动播放 → 记录聆听。**
音频指标通过不等于临床疗效；本项目尚未完成患者研究或产品临床验证。

## 快速开始

需要 Rust stable。离线作曲无需模型或 API：

```bash
cargo build --release --locked
./target/release/tmusic generate --style drift --duration 60 --tempo 54 --seed 42 -o drift.wav
./target/release/tmusic process -i drift.wav -o drift_listening.wav --center 6000
```

`generate` 保存原始 WAV 和生成参数；`process` 保存处理后的 WAV、检查结果及
SHA-256。退出码：`0` 成功、`1` 执行错误、`2` 音频检查未通过。
使用 `tmusic --help` 查看分析、滤波器设计、耳鸣匹配及模拟命令。

## 项目结构

| 路径 | 职责 |
|---|---|
| `src/` | DSP、WAV、重采样、匹配流程、离线作曲和可选 ACE-Step 适配器 |
| `app/src/` | React 创作工作室、音乐库、播放器和临床配置界面 |
| `app/src-tauri/src/` | 桌面状态、播放、匹配、音乐库、临床命令与持久化 |
| `tests/`、`app/tests/` | 信号验证、HTTP 协议、持久化及界面回归测试 |
| `scripts/` | 可选模型服务安装与生成验证 |
| `artifacts/validation/` | 小型、可追溯的历史技术测量记录 |

- [桌面开发与数据说明](app/README.md)
- [生成器、ACE-Step 配置与历史测量](docs/generation.md)
- [音频处理、验证指标与匹配流程](docs/audio-engine.md)
- [音乐生成实验室：候选筛选、偏好学习、同源对照](research/README.md)

生成源始终保留原始音频；播放和导出时按当前参与者配置重新处理。研究参与者
的生成和导出由临床模式管理，界面不显示分组。PIN 仅是界面锁，不是数据加密。

## 验证

```bash
cargo test --locked --features generation-http
cargo clippy --locked --all-targets --features generation-http -- -D warnings
python3 -m unittest discover -s research/tests -v
cd app
npm ci
npm run build
npx playwright install chromium
npm test
```

HTTP 测试使用本地模拟服务器；浏览器测试模拟 Tauri IPC。真实桌面设备的音频输出、
声压级校准和长时间运行仍需单独验证。模型服务是可选组件，普通开发无需下载权重。
CI 同时检查核心 Rust、浏览器界面和 Windows 桌面编译。

## 边界

- 输入格式为 WAV；其他格式需先解码。无损输出用于保留陷波结果。
- 生成时长为 30–300 秒；离线作曲不是神经模型。
- LUFS / dBTP 是数字音频指标，不表示耳机实际声压级。
- 历史模拟和 GPU 测量是技术验证，不能推导患者获益或跨硬件可复现性。
- 发布前需另行确认模型与数据许可、临床证据以及目标市场的产品要求。
