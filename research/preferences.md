# 个人音乐偏好探索

这个离线原型从预先列出的生成配方中推荐一个候选，供“下一首生成什么”实验使用。
仅依赖 Python 3 标准库。示例中的两个参与者与所有评分均为**合成数据**。

在仓库根目录执行：

```bash
python3 -m research.preferences --input research/examples/preferences.json --participant synthetic-alice --seed 42
python3 -m research.preferences --input research/examples/preferences.json --participant synthetic-bob --seed 42
python3 -m unittest discover -s research/tests -p 'test_preferences.py' -v
```

输出包含选中的完整配方、候选评分、个人反馈数量、排除原因和随机种子。
同一 Python 实现和版本下，相同输入与种子产生相同输出；候选按 ID 排序。
反馈按输入顺序读取，复现实验时保留原始 JSON 和运行环境。
`--seed` 控制推荐探索，不是音频生成种子。

## 输入

- `catalog`：非空配方数组。每项必须包含唯一 `recipe_id`、`provider`
  （`composer` / `ace_step`）、`style`（`drift` / `focus` / `unwind`）、
  整数 `tempo_bpm`（45–100）、整数 `duration_s`（30–300）和 `prompt`
  （最多 500 字符）。`composer` 不支持文字提示，`prompt` 必须为空或仅包含
  空白字符；`ace_step` 可使用非空提示。保持配方 ID 与内容绑定；修改内容时使用新 ID。
- `feedback`：可为空。每项包含 `participant_id`、存在于目录中的 `recipe_id`、
  1–5 的有限数值 `rating` 和布尔值 `discomfort`。
- `synthetic`：可选布尔标记；示例为 `true`。

每条反馈都视为一次独立评分，不按配方去重；调用方应排除重复导入的事件。
未知字段、重复配方 ID、重复 JSON 字段、未知配方引用以及非法评分会报错，
CLI 以状态码 2 退出。整个文件先验证，随后只用指定参与者的反馈计算。

## 评分如何工作

对每个未被该参与者报告不适的配方，将评分变换为 `x = (rating - 1) / 4`，
设置 `alpha = 2 + Σx`、`beta = 2 + Σ(1 - x)`。
`regularized_score = alpha / (alpha + beta)` 在数据少时向 0.5 收缩。
使用指定种子从各配方的 `Beta(alpha, beta)` 分布抽样，选择抽样值最大的配方。
因此评分最高的配方并非每次都会获选，未评分配方也有探索机会。
这里使用分数计数作启发式更新；分布并非经验证的临床后验或置信区间。

任何一次 `discomfort=true` 都会排除该参与者的相应配方，之后的高分不会撤销。
其他参与者的评分与不适报告不会改变本人的结果。所有候选均被排除时直接报错。

## 研究与产品边界

这是偏好探索，不能由偏好评分推断耳鸣改善，也不提供因果结论。
重复评分的相关性、选择偏差、期待效应和偏好随时间变化均未建模。
后续比较固定配方与自适应推荐时，应另行设计预注册评价、独立结果指标和
选择概率日志；当前抽样值并不是该配方被选中的概率，不能直接用于离线因果估计。

原型只写 JSON 到标准输出，不保存参与者资料、不调用模型、不自动播放。
目录应由调用方提供可用配方；本程序不探测 ACE-Step 服务是否可用。
它不读取或调整耳鸣频率、陷波、增益或治疗分组。建议若接入现有产品，
仍需经过其生成、音频验证、按当前配置处理和用户主动播放流程。
偏好分数及“未报告不适”均不构成播放安全证明。
