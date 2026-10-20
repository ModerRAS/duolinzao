#!/usr/bin/env bash
# 改已构建的通用 APK：替换 assets/server.json → zipalign → apksigner 重签 → 生成 version.json。
# 用法：scripts/inject-server-url.sh https://app.example.com /path/to/duolinzao.apk [out.apk]
# 产物：out.apk + 同目录 version.json（versionCode/versionName 从通用 APK 原样读取，不覆盖）。
# 密钥不进 git；签名用部署方自己的 keystore。
set -euo pipefail

if [ $# -lt 2 ]; then
  echo "用法: $0 <BASE_URL> <通用APK> [输出APK]" >&2
  exit 1
fi
BASE_URL="$1"
APK="$2"
OUT="${3:-${APK%.apk}-site.apk}"
OUT_DIR="$(cd "$(dirname "$OUT")" && pwd)"
OUT_BASE="$(basename "$OUT")"

# SDK 工具自动入 PATH（部署机只需设 ANDROID_HOME 或 ANDROID_SDK_ROOT）。
SDK_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/AppData/Local/Android/Sdk}}"
if [ -d "$SDK_HOME" ]; then
  latest_bt="$(ls -d "$SDK_HOME"/build-tools/*/ 2>/dev/null | sort -V | tail -1)"
  [ -n "$latest_bt" ] && PATH="$latest_bt:$PATH"
  [ -d "$SDK_HOME/cmdline-tools/latest/bin" ] && PATH="$SDK_HOME/cmdline-tools/latest/bin:$PATH"
  [ -d "$SDK_HOME/platform-tools" ] && PATH="$SDK_HOME/platform-tools:$PATH"
fi

# Windows 上这些是 .bat/.exe，command -v 不认：逐个找本体或 .bat。
resolve() {
  command -v "$1" 2>/dev/null || command -v "$1.bat" 2>/dev/null || command -v "$1.exe" 2>/dev/null || return 1
}
ZIPALIGN="$(resolve zipalign || echo '')"
APKSIGNER="$(resolve apksigner || echo '')"
APKANALYZER="$(resolve apkanalyzer || echo '')"
AAPT="$(resolve aapt || echo '')"
UNZIP="$(resolve unzip || echo '')"
SHA="$(resolve sha256sum || echo '')"
if [ -z "$UNZIP" ] || [ -z "$ZIPALIGN" ] || [ -z "$SHA" ]; then
  echo "缺少工具：需要 unzip + zipalign + sha256sum（已自动尝试 ANDROID_HOME 下的 SDK）" >&2
  exit 1
fi
if [ -z "$APKANALYZER" ] && [ -z "$AAPT" ]; then
  echo "缺少工具：需要 apkanalyzer 或 aapt（读通用 APK 版本号用）" >&2
  exit 1
fi
# 有些 Windows 只有 python3 的商店占位符：以能实际跑起来为准。
PY=""
for cand in python3 python; do
  if "$cand" -c 'import sys; sys.exit(0)' >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "缺少工具: python3/python" >&2; exit 1; }

if [ -n "${KEYSTORE:-}" ] && [ -z "${KEY_ALIAS:-}" ]; then
  echo "已设 KEYSTORE 但未设 KEY_ALIAS" >&2
  exit 1
fi

# Windows git-bash 调 .bat/.exe 工具时 MSYS 路径转换会抽风：转成 C:\… 形式。
to_tool_path() {
  if [ -n "${MSYSTEM:-}" ] && command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}

# 读通用 APK 版本（部署方不覆盖）。
if [ -n "$APKANALYZER" ]; then
  VER_CODE="$("$APKANALYZER" manifest version-code "$(to_tool_path "$APK")" | tr -d ' \r\n')"
  VER_NAME="$("$APKANALYZER" manifest version-name "$(to_tool_path "$APK")" | tr -d ' \r\n')"
else
  VER_CODE="$("$AAPT" dump badging "$(to_tool_path "$APK")" | sed -n "s/.*versionCode='\([^']*\)'.*/\1/p" | head -1)"
  VER_NAME="$("$AAPT" dump badging "$(to_tool_path "$APK")" | sed -n "s/.*versionName='\([^']*\)'.*/\1/p" | head -1)"
fi
echo "通用 APK: versionCode=$VER_CODE versionName=$VER_NAME"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp "$APK" "$WORK/base.apk"
cd "$WORK"
"$UNZIP" -q -o base.apk 'assets/server.json'

# 只改 baseUrl 并关 mock（部署包连自家真后端）；用 python 写回包内，不依赖 zip 命令。
BASE_URL="$BASE_URL" "$PY" - <<'EOF'
import json, os, zipfile
p = "assets/server.json"
with open(p, encoding="utf-8") as f:
    data = json.load(f)
data["baseUrl"] = os.environ["BASE_URL"].rstrip("/")
data["useMock"] = False
with open(p, "w", encoding="utf-8") as f:
    json.dump(data, f, ensure_ascii=False, indent=2)
    f.write("\n")
print(open(p, encoding="utf-8").read())
# 重写包（去掉旧条目再写入，避免出现重复的 assets/server.json）。
with zipfile.ZipFile("base.apk", "r") as zin:
    items = [(i, zin.read(i.filename)) for i in zin.infolist() if i.filename != p]
with zipfile.ZipFile("base.apk", "w", zipfile.ZIP_DEFLATED) as zout:
    for info, data in items:
        zout.writestr(info, data)
    zout.write(p, p)
EOF

"$ZIPALIGN" -f 4 "$(to_tool_path base.apk)" "$(to_tool_path aligned.apk)"

if [ -n "${KEYSTORE:-}" ]; then
  # 部署方自己的 keystore（密码经环境传入，不进仓库）。
  "$APKSIGNER" sign --ks "$(to_tool_path "$KEYSTORE")" --ks-pass "env:KEYSTORE_PASS" \
    ${KEY_ALIAS:+--ks-key-alias "$KEY_ALIAS"} \
    ${KEY_PASS:+--key-pass "env:KEY_PASS"} \
    --out "$(to_tool_path "$OUT_DIR/$OUT_BASE")" "$(to_tool_path aligned.apk)"
else
  echo "未设 KEYSTORE：输出未签名包（仅供检查 server.json；发布前必须用部署方 keystore 重签）" >&2
  cp aligned.apk "$OUT_DIR/$OUT_BASE"
fi

# version.json：版本号沿用通用 APK 原值，url 同站，sha256 为最终 APK 摘要。
SHA="$("$SHA" "$OUT_DIR/$OUT_BASE" | cut -d' ' -f1)"
URL_PATH="/app/$OUT_BASE"
"$PY" - "$(to_tool_path "$OUT_DIR/version.json")" "$VER_CODE" "$VER_NAME" "$URL_PATH" "$SHA" <<'EOF'
import json, sys
out, code, name, url, sha = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]
with open(out, "w", encoding="utf-8") as f:
    json.dump({"versionCode": code, "versionName": name, "url": url, "sha256": sha}, f, ensure_ascii=False, indent=2)
    f.write("\n")
EOF
echo "已写 $OUT_DIR/$OUT_BASE"
echo "已写 $OUT_DIR/version.json"
echo "请把这两个文件放到站点静态目录 {static_dir}/app/（/app/* 缺文件必须 404，不能回退 index.html）。"
