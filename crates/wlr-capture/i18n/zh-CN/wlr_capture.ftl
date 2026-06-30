# 简体中文翻译（界面文本）— wlr-chooser。
tab-all = 全部
tab-windows = 窗口
tab-outputs = 屏幕
filter-hint = 筛选…
screen-label = 屏幕 { $name }
loading = …
show-system = 系统窗口
error = wlr-chooser: { $error }
pip-gone = 源窗口已关闭
overlay-region-hint = 拖动以选择 · Esc 取消
overlay-pick-hint = 点击以取色 · Esc 取消
overlay-magnify-hint = 滚动以缩放 · Esc 退出
draw-hint = 绘图 · h 帮助 · Esc 离开
draw-passthrough-hint = 点击穿透 — 按 Caps Lock 重新绘图
draw-text-hint = 输入文字 · Enter 放置 · Esc 取消
draw-spotlight-hint = 聚光灯 · 拖出形状或移动光标 · 滚轮/i k 大小 · 倾斜/j l 暗度 · 松开 Shift 结束
draw-tool-pen = 画笔
draw-tool-rect = 矩形
draw-tool-mask = 遮罩（实心框）
draw-tool-arrow = 箭头
draw-tool-text = 文字
draw-tool-eraser = 橡皮擦
draw-palette-title = 颜色
draw-help-title = wlr-draw — 按键
draw-help-tools = 工具（画笔、直线、矩形、椭圆、箭头、文字、橡皮擦）
draw-help-color = 调色板
draw-help-undo = 撤销 / 重做
draw-help-width = 笔触宽度
draw-help-clear = 全部清除
draw-help-visibility = 隐藏 / 显示
draw-help-help = 本帮助
draw-help-leave = 离开绘图模式
draw-help-constrain = 按住：约束（圆形 / 正方形 / 轴向）
draw-help-spotlight = 按住：聚光灯（在形状 / 光标周围变暗）
draw-help-spotlight-tune = 聚光灯：滚轮/i,k 大小 · 倾斜/j,l 暗度
draw-help-passthrough = Caps Lock：点击穿透到应用
draw-help-draw = 左键拖动：绘制
draw-help-snap = 画笔，按住不动：吸附为规整形状
draw-help-text = 文字：点击、输入、Enter
tray-status-drawing = 绘图中
tray-status-idle = 空闲（点击穿透）
tray-toggle = 切换绘图模式
tray-clear = 清除
tray-undo = 撤销
tray-shortcuts = 快捷键
tray-quit = 退出
tray-autostart = 登录时启动
draw-help-move = 方向键：移动所选 · Ctrl：大步移动
draw-tool-move = 移动
draw-help-freeze = 空格：冻结屏幕以标注
draw-help-rightmove = 右键拖动：移动元素
draw-help-save = 保存带标注的截图

# Capture capability — graceful degradation on compositors without window
# capture (foreign-toplevel image-capture source; wlroots < 0.20 / Sway < 1.12).
capture-no-window = 此合成器无法捕获单个窗口。窗口捕获需要 wlroots >= 0.20（Sway >= 1.12）；wlroots 0.19 / Sway 1.11 仅支持屏幕捕获。运行 `wlr-peek doctor` 查看你的合成器支持哪些功能。
draw-capture-unavailable = 此处无法进行屏幕捕获 — 冻结和保存已禁用。
