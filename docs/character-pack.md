# 角色包：把自己画的角色放进 KuroCat

角色包是一个 ZIP 文件，里面有一份 `character.json` 和若干张透明背景的 PNG 图。导入后，角色会出现在右键菜单「角色外观」里，和小黑猫一样跟着键盘和鼠标动，输密码时也会自动换成捂眼的图。

先试试示例包：[examples/character-packs/tuanzi.zip](../examples/character-packs/tuanzi.zip)。图片和 `character.json` 的原样在 [examples/character-packs/tuanzi](../examples/character-packs/tuanzi) 文件夹里，可以照着改。

## 导入、导出和删除

- **导入**：右键桌宠 →「外观与互动」→「＋ 导入角色包」，选好 ZIP 后会自动换成这个角色。再次导入同一个 `id` 的包会覆盖旧的，改图后重新打包导入即可。
- **导出**：选中导入的角色后点「导出」，会把它重新打包成 ZIP，方便换电脑或分享。
- **删除**：选中导入的角色后点「删除」，再点一次确认。
- 角色包保存在 KuroCat 的数据文件夹（和 `settings.json` 同一处）里的 `characters` 文件夹中，升级软件不会丢。
- 导入的角色只用 2D 画面显示。选中它时，即使设成了 3D 也会改用 2D；换回小黑猫等内置猫时，会恢复原来的设置。
- 用 AI 捏猫（MCP）的 `list_cats` 能看到导入的角色，`switch_cat` 能切换到它，`delete_cat` 能删除它。

## 打包方法

把 `character.json` 和图片放在同一个文件夹里，然后压缩成 ZIP：

- Mac：在访达里右键这个文件夹，选「压缩」。
- Windows：右键文件夹，选「压缩为 ZIP 文件」。

`character.json` 放在 ZIP 最外层，或放在 ZIP 里唯一的那个文件夹中都可以。一个 ZIP 只能放一个角色。

## character.json

```json
{
  "formatVersion": 1,
  "id": "tuanzi",
  "name": "团子",
  "emoji": "🍡",
  "author": "你的名字",
  "scale": 1.0,
  "offsetX": 0,
  "offsetY": 0,
  "showKeyboard": true,
  "purr": false,
  "accentColor": "#ffb3c1",
  "accentSideColor": "#e0708a",
  "states": {
    "idle": { "frames": ["idle.png", "idle-breathe.png"], "fps": 1.5 },
    "blink": "blink.png",
    "typingLeft": ["type-left-1.png", "type-left-2.png"],
    "typingRight": ["type-right-1.png", "type-right-2.png"],
    "mouse": "mouse.png",
    "mouseClick": "click.png",
    "privacy": "privacy.png",
    "sleep": "sleep.png",
    "pet": "pet.png"
  }
}
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | 是 | 菜单里显示的名字 |
| `states.idle` | 是 | 待机时的图，其他状态没给图时都会用它 |
| `id` | 否 | 只能用英文字母、数字、`-`、`_`。不写时用 ZIP 的文件名。和内置猫或 AI 捏的猫重名时，会自动在后面加 `-pack` |
| `emoji` | 否 | 菜单里名字前面的图标，默认 🎨 |
| `author` | 否 | 作者 |
| `scale` | 否 | 角色大小，默认 1，可填 0.3 到 2 |
| `offsetX` / `offsetY` | 否 | 往右 / 往下挪多少（单位是画面上的点，画面宽约 420），默认 0 |
| `showKeyboard` | 否 | 是否显示 KuroCat 自带的键盘和鼠标，默认显示。图里自己画了桌面时填 `false` |
| `purr` | 否 | 摸头时是否发出呼噜声，默认不发 |
| `accentColor` / `accentSideColor` | 否 | 按下的键帽顶面 / 侧面颜色，写成 `#RRGGBB` |
| `formatVersion` | 否 | 格式版本，目前是 1 |

## 状态

| 状态 | 什么时候显示 | 没给图时用 |
| --- | --- | --- |
| `idle` | 待机 | （必填） |
| `blink` | 待机时每 3 到 6 秒眨一次眼 | `idle` |
| `typingLeft` / `typingRight` | 按下画面左半边 / 右半边的键 | `typing` → `idle` |
| `typing` | 按键（不分左右） | `idle` |
| `mouse` | 移动鼠标 | `idle` |
| `mouseClick` | 按住鼠标键 | `mouse` → `idle` |
| `privacy` | 正在输入密码（开启了「输密码自动闭眼防偷窥」时） | `blink` → `idle` |
| `sleep` | 约 28 秒没有操作后睡着 | `blink` → `idle` |
| `pet` | 用鼠标来回摸角色的头 | `idle` |

同时满足多个时，按这个顺序取第一个：输密码、按住鼠标、打字、被摸、睡觉、移动鼠标、眨眼、待机。

每个状态可以写成三种样子：

- 一张图：`"blink": "blink.png"`
- 几张图：`"typingLeft": ["a.png", "b.png"]`。打字状态每按一次键换下一张；其他状态按每秒 8 张循环播放
- 带速度的几张图：`"idle": { "frames": ["a.png", "b.png"], "fps": 2 }`，`fps` 是每秒几张，可填 1 到 30

图片路径相对于 `character.json`，可以放在子文件夹里，比如 `"frames/idle-1.png"`。

## 画图建议

- 用透明背景的 PNG，同一个角色的每张图尺寸保持一致。推荐 600×600 像素。
- 图会等比缩放到画面中约 280×280 的方框里，方框底边中点对准键盘中间偏下的位置。开着键盘时，图的下面大约十分之一会被键盘挡住。把手画在离图底边八分之一到十分之一的高度，看起来就像搭在键盘上；打字的图把手画低一点，正好按在键盘边上。
- 角色主体尽量画在图的中间。桌宠只在角色中间的椭圆区域和键盘区域响应点击和拖动，画在四角的部分点上去会穿透到后面的窗口。
- 打字的图可以左右手各画一张，再各加一张稍有变化的，这样连续打字时会更生动。

## 限制

- 只支持 PNG 图片。
- 每张图不超过 8 MB，所有图加起来不超过 40 MB，每个状态最多 60 张。
- 状态名写错或缺图时，桌宠上会提示是哪一项出了问题（提示太长被截断时，把鼠标移到提示上能看到全文）。导入失败不会影响已经导入的角色。
