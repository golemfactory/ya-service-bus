
fail() {
	printf "%s\n" "$1" >&2
	exit 1
}

not_empty() {
	test -z "$1" && fail "expected $2"
}


not_empty "$GITHUB_REF" GITHUB_REF
not_empty "$OS_NAME" OS_NAME


if [ "$OS_NAME" = "ubuntu" ]; then
  OS_NAME=linux
  target=x86_64-unknown-linux-musl/
  exe=""
elif [ "$OS_NAME" == "macos" ]; then
  OS_NAME=osx
elif [ "$OS_NAME" == "windows" ]; then
  exe=".exe"
else
  fail "unknown os name: $OS_NAME"
fi

TAG_NAME="${GITHUB_REF##*/}"

generate_asset() {
  local asset_type=$1
  local bins="$2"
  local asset_name="${asset_type}-${OS_NAME}-${TAG_NAME}"
  local TARGET_DIR=releases/${asset_name}
  mkdir -p "$TARGET_DIR"
  for component in $bins; do
    strip -x target/${target}release/${component}${exe}
  done
  for bin in $bins; do
    cp "target/${target}release/${bin}${exe}" "$TARGET_DIR/"
  done
  if [ "$OS_NAME" = "windows" ]; then
    echo "::set-output name=artifact::${asset_name}.zip"
    echo "::set-output name=media::application/zip"
    (cd "$TARGET_DIR" && 7z a "../${asset_name}.zip" * )
  else
    echo "::set-output name=artifact::${asset_name}.tar.gz"
    echo "::set-output name=media::application/tar+gzip"
    (cd releases && tar czf "${asset_name}.tar.gz" "${asset_name}")
    du -h "releases/${asset_name}.tar.gz"
  fi
}

generate_asset "$1" "$2"
