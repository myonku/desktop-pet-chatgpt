"""Semantic groups and preview labels for the white-dragon source clips."""

# The first present variant in each group is its default, including partial builds.
ACTION_GROUPS = {
    "idle": (
        ("rest", "idle", "待机"),
        ("look-around", "idle2", "左右看"),
        ("read", "idle03", "趴着看书"),
        ("nap", "idle04", "侧躺打盹"),
        ("gaming", "idle05", "打游戏"),
        ("tail-play", "idle06", "玩尾巴"),
        ("sleepy", "variant-14", "犯困"),
        ("cookie", "variant-17", "吃饼干"),
        ("coins", "variant-24", "吃金币"),
    ),
    "interact": (
        ("wave", "interact", "招手"),
        ("hug", "variant-08", "来抱抱"),
        ("heart", "variant-20", "比心"),
        ("invite", "variant-22", "一起来吧"),
        ("scroll", "variant-21", "魔法卷轴"),
        ("fire", "variant-25", "喷火龙"),
        ("crystal", "variant-26", "魔法结晶"),
    ),
    "drag": (("held", "drag", "拖拽"),),
    "land": (("land", "land", "落地"),),
    "move.left": (
        ("run", "move-left", "向左跑"),
        ("walk", "move01left", "向左走"),
        ("fly", "move03left", "向左飞"),
    ),
    "move.right": (
        ("run", "move-right", "向右跑"),
        ("walk", "move01right", "向右走"),
        ("fly", "move03right", "向右飞"),
    ),
    "dock.left": (("hold", "edge-left", "左侧吸边"),),
    "dock.right": (("hold", "edge-right", "右侧吸边"),),
    "peek.left": (("peek", "peek-left", "左侧探头"),),
    "peek.right": (("peek", "peek-right", "右侧探头"),),
}

PREVIEW_ONLY = {
    "quota": "看笔记",
    "work01": "记笔记",
    "work02": "敲键盘",
    "work03": "检查报告",
    "work04": "查资料",
    "work05": "放大镜",
    "variant-09": "考虑一下",
    "variant-18": "欢呼",
    "variant-04": "新点子",
    "variant-05": "得意",
    "variant-13": "生气",
}


def bind_white_dragon(manifest):
    clips = {clip["id"]: clip for clip in manifest["clips"]}
    bindings = {}
    for behavior, group in ACTION_GROUPS.items():
        variants = {}
        for name, clip_id, label in group:
            if clip_id in clips:
                clips[clip_id]["preview_name"] = label
                variants[name] = clip_id
        if variants:
            bindings[behavior] = {"default": next(iter(variants)), "variants": variants}
    for clip_id in clips.keys() & PREVIEW_ONLY.keys():
        clips[clip_id]["preview_name"] = PREVIEW_ONLY[clip_id]
    manifest["bindings"] = bindings
