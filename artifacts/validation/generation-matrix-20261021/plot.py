"""Recreate the figure from admission.csv; requires matplotlib."""
import csv
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

root = Path(__file__).resolve().parent
with (root / "admission.csv").open() as stream:
    rows = list(csv.DictReader(stream))
recipes = list(dict.fromkeys(row["recipe_id"] for row in rows))
centers = [1000, 3000, 6000, 10000]
labels = [name.replace("composer-", "Composer · ").replace("ace-", "ACE · ")
          .replace("-baseline", " · baseline").replace("-overtone", " · overtones")
          for name in recipes]
fig, axes = plt.subplots(1, 2, figsize=(11, 6), sharey=True)
for ax, split in zip(axes, ("development", "holdout")):
    indexed = {(r["recipe_id"], int(r["center_hz"])): r for r in rows if r["split"] == split}
    rates = [[float(indexed[(recipe, center)]["admission_rate"]) for center in centers]
             for recipe in recipes]
    ax.imshow(rates, vmin=0, vmax=1, cmap="Blues", aspect="auto")
    for y, recipe in enumerate(recipes):
        for x, center in enumerate(centers):
            row = indexed[(recipe, center)]
            ax.text(x, y, f'{row["admitted"]}/{row["attempted"]}', ha="center", va="center",
                    color="white" if rates[y][x] > 0.6 else "#142539", fontsize=11)
    ax.set_xticks(range(4), ["1", "3", "6", "10"])
    ax.set_yticks(range(len(labels)), labels)
    ax.set_xlabel("Processing center (kHz)")
    ax.set_title(split.capitalize(), fontsize=13, pad=12)
    ax.tick_params(length=0, pad=8)
    for spine in ax.spines.values():
        spine.set_visible(False)
fig.suptitle("Technical admission of 30-second music sources", x=0.035, ha="left", fontsize=17)
fig.text(0.035, 0.91, "Admitted / attempted sources · all checks pass, matching hashes, no warnings",
         fontsize=10, color="#435165")
fig.text(0.035, 0.035,
         "72 sources, 288 repeated checks. Four seeds per recipe and split; centers reuse each source.\n"
         "Technical screening only. No listening preference or clinical outcomes were measured.",
         fontsize=9, color="#435165")
fig.subplots_adjust(left=0.255, right=0.975, top=0.82, bottom=0.17, wspace=0.18)
fig.savefig(root / "admission.png", dpi=180, facecolor="white")
