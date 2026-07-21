# PTAM2 图集与 DDS 补丁算法

## 普通图集

每个网格单元的宽高取所有输入贴图的最大宽高。布局模式决定行列数：

- `auto`：列数为 `ceil(sqrt(n))`
- `horizontal`：`n × 1`
- `vertical`：`1 × n`
- `grid`：使用指定列数，行数向上取整

所需画布尺寸为：

```text
width  = columns × cell_width  + (columns - 1) × padding
height = rows    × cell_height + (rows - 1)    × padding
```

普通图集使用 alpha overlay，行为与旧版 `alpha_composite` 一致。

## 底图槽位

槽位尺寸取第一张待合并贴图的尺寸。所有待合并贴图必须同尺寸，且底图宽高必须能被槽位宽高整除：

```text
columns = base_width  / slot_width
rows    = base_height / slot_height
```

槽位从 1 开始按行优先编号。替换时采用像素精确覆盖，而非 alpha 混合，因此透明像素也会覆盖底图对应像素。

## DDS 二进制补丁

仅在以下条件全部满足时启用：

- 底图和输出均为 DDS
- 最终画布尺寸与底图一致
- 底图为二维单层纹理且只有 1 级 mipmap
- 底图格式为 BC1、BC2、BC3 或 BC7
- 底图和槽位宽高都是 4 像素的整数倍
- 所有待合并贴图都已分配唯一槽位

DDS 压缩以 4×4 像素块存储。对槽位 `(column, row)`，每个压缩块行的目标偏移为：

```text
offset = header_size
       + ((slot_y / 4 + block_row) × base_blocks_per_row + slot_x / 4)
       × block_size
```

BC1 每块 8 字节；BC2、BC3、BC7 每块 16 字节。

每个替换槽位有两条路径：

1. `direct-copy`：来源是相同格式、相同槽位尺寸的 DDS，直接复制第 0 级 mip 的压缩块。
2. `native-reencode`：来源格式或尺寸不匹配，使用与底图相同的 BC 格式在进程内编码。

所有替换块先并行准备。写入完成后，程序逐字节比较原始与目标缓存，任何目标槽位范围之外的变化都会使导出失败。

## sRGB

`BC7_UNORM_SRGB` 和其他 sRGB BC 格式只改变 DDS 的格式标记；输入 RGBA8 通道值直接作为 sRGB 样本压缩，不执行额外 gamma 变换。这等价于旧版在 `texconv` 中使用 `-srgbi` 避免重复 gamma 转换的意图。

## mipmap 策略

当前导出与补丁都只生成/接受 1 级 mipmap。带 mipmap 的底图会回退到全量重编码，防止只更新最高级纹理而在较低 mip 中残留旧图。
