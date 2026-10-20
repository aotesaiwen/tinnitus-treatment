# 同源对照材料

同一首歌同时制作 `notch`、`passthrough`、`sham`，固定默认响度、峰值和陷波宽度。
输入音频只读。三种条件都必须通过 DSP 检查、没有警告、来源和输出哈希一致，
且输出响度极差不超过 0.2 LU，才发布试听目录。

```bash
python3 -m research.paired --binary target/release/tmusic \
  --source source.wav --center 6000 --sham-center 1500 \
  --seed 42 --output artifacts/audio/paired-demo
```

- `listening/`：中性文件名和 `schedule.json`。六种排列完整覆盖三段音频的顺序，
  每个条件在每个位置出现两次，有向相邻组合也各出现两次。
- `private/`：条件映射、处理报告、参数及来源哈希。仅供研究人员保留，不能连同
  试听目录一起发给评估者。该目录名不是访问控制或加密。
- `summary.json`：整体是否通过、响度差和映射文件位置。
- 已存在的输出目录会被拒绝。未通过时退出码为 `2`，仅留下审计材料，
  不生成 `listening/`；执行错误退出码为 `1`。

DSP 报告中的 `output_path` 是处理时的暂存路径；通过后音频移入 `listening/`，
最终位置和哈希以 `schedule.json` 为准。

这能控制“不同条件用了不同音乐”这一混杂，但不是完整临床试验设计：
六种顺序不等于六个独立样本，也不能排除被试听者听出处理差异。
正式研究还需指定独立的音乐/参与者样本数、分配与隐藏方法、携带效应处理、
预设结局和分析方案。这个工具不会分配真实参与者、启动播放或计算疗效。
