# PTAM2

PTAM2 是一个使用 **Rust + Tauri 2** 重构的桌面贴图拼接工具。它可以把多张贴图排列为 PNG/DDS 图集，也可以在已有 DDS 底图上按槽位替换贴图，并保持未覆盖区域的原始压缩字节不变。

## 主要功能

- 读取 DDS、PNG、JPG、BMP、TGA、TIFF、WebP
- 自动网格、横向、纵向、固定列数布局
- 自动画布、常用方形画布、自定义宽高与间距
- 一键统一贴图尺寸（Lanczos3 重采样）
- DDS/普通图片底图槽位模式，支持拖放、移动与交换槽位
- 导出 PNG、DXT5、BC7 Linear、BC7 sRGB
- 可选 Fast / Normal / Slow DDS 压缩质量
- 同时导出 JSON 坐标表
- 可选界面背景和面板透明度，自动保存窗口位置与大小
- DDS 导出诊断：显示每个槽位采用二进制直拷贝还是原生重编码

## 重构与算法优化

旧版 Python/PySide6 实现依赖 Pillow，并为不匹配的 DDS 槽位逐个启动 `texconv.exe`。PTAM2 将核心处理迁移到 Rust：

1. **原生 DDS 管线**：使用 `image_dds`、`ddsfile` 和 Intel ISPC Texture Compressor，在进程内编解码 BC1/BC2/BC3/BC7，不再随包分发或启动 `texconv.exe`。
2. **并行槽位预处理**：所有需要重编码的槽位通过 Rayon 并行压缩，然后顺序写入目标块。
3. **零损替换优先**：来源 DDS 与底图的格式、尺寸一致时，直接复制 BC 压缩块，不重复有损编码。
4. **严格字节保护**：补丁导出先复制原始 DDS，只改目标槽位对应的 BC 块；写入前会验证所有变化都位于允许区间。
5. **原子写入**：PNG、DDS 和 JSON 都先写同目录临时文件，再替换目标，避免中途中断留下损坏文件。
6. **预览与导出解耦**：预览最长边限制为 2048px，降低 WebView 传输与显存占用；最终导出始终使用完整分辨率。
7. **内存保护**：对异常画布尺寸、整数溢出和超大像素总量进行前置校验。

更详细的约束与流程见 [算法说明](docs/ALGORITHM.md)。

## 开发

环境要求：

- Rust 1.88 或更高版本
- Node.js 20 或更高版本
- pnpm 10 或更高版本
- Windows 10/11 与 WebView2（Windows 构建）

```powershell
pnpm install
pnpm tauri dev
```

运行检查：

```powershell
pnpm build
cd src-tauri
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

生成安装包：

```powershell
pnpm tauri build
```

Windows 安装包会生成到 `src-tauri/target/release/bundle/`。

## JSON 格式

```json
{
  "width": 2048,
  "height": 2048,
  "items": [
    {
      "textureId": 1,
      "name": "icon.png",
      "path": "D:\\textures\\icon.png",
      "x": 0,
      "y": 0,
      "width": 256,
      "height": 256,
      "slot": 1
    }
  ]
}
```

`slot` 在普通图集模式中为 `null`，在底图槽位模式中为从 1 开始的槽位编号。

## 第三方组件

DDS 压缩由 MIT 许可的 `image_dds` 与 `intel_tex_2` 提供；应用不包含旧版的 `texconv.exe`。项目自身尚未声明开源许可证。
