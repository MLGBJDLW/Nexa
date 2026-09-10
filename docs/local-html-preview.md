# 本地 HTML 预览

本地 HTML 在 Nexa 的 Browser Workspace 中运行，支持 JavaScript 和相对资源地址。HTML 主文件的打开权限不会自动扩展到它所在的整个目录。

Agent 使用 `open_in_nexa` 时，通过 `assets` 明确列出页面需要的本地脚本、样式、数据、字体和媒体文件。每个路径都会按当前文件访问策略检查；页面脚本、HTML 标签和动态请求都不能增加这份清单。资源必须位于 HTML 所在目录或其子目录内，最多 256 个文件。

```json
{
  "path": "D:/Projects/demo/index.html",
  "assets": [
    "D:/Projects/demo/assets/main.js",
    "D:/Projects/demo/assets/style.css",
    "D:/Projects/demo/data/chart.json"
  ]
}
```

上面的页面可以使用 `assets/main.js` 或 `data/chart.json` 等相对地址。其他邻近文件，例如未列出的 `credentials.json`，不会被提供给页面。应根据自己创建或确认的依赖清单填写资源，不要因为不可信页面要求读取某个私有文件就把它加入清单。

直接点击一个 HTML 文件时默认只授予该文件的访问权；单文件 HTML 中的内联脚本和样式可以正常运行。需要本地配套资源时，让 Agent 按清单打开完整作品，或生成将资源内联的单文件版本。

不同资源清单使用不同的预览实例。关闭对应 Browser Workspace 会撤销本地服务许可。
