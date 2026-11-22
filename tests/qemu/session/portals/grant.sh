#!/bin/sh
# Construct both Flatpak refs locally using the guest's own ABI and no external remote.
set -eu
binary=$1
work=$(mktemp -d)
cd "$work"
mkdir -p runtime/files runtime/usr/lib runtime/usr/bin app/files/bin app/export
cat >runtime/metadata <<'END'
[Runtime]
name=org.joelinux.GateRuntime
runtime=org.joelinux.GateRuntime/x86_64/stable
sdk=org.joelinux.GateRuntime/x86_64/stable
END
# The tiny runtime needs the loader and the transitive ELF dependencies of this client.
ldd "$binary" | awk '$2 == "=>" && $3 ~ /^\// {print $3} $1 ~ /^\// {print $1}' |
while read -r library; do cp -L "$library" runtime/usr/lib/; done
ln -s lib runtime/usr/lib64
cp "$binary" app/files/bin/choose
cat >app/metadata <<'END'
[Application]
name=org.joelinux.FileGrantTest
runtime=org.joelinux.GateRuntime/x86_64/stable
sdk=org.joelinux.GateRuntime/x86_64/stable
command=choose
END
flatpak build-export --runtime repo runtime stable
flatpak build-export repo app stable
for ref in org.joelinux.FileGrantTest org.joelinux.GateRuntime; do
    if flatpak info --user "$ref" >/dev/null 2>&1; then
        echo "Fixture ref already installed: $ref" >&2
        exit 1
    fi
done
remote=mundane-gate-$$
flatpak remote-add --user --no-gpg-verify "$remote" "$work/repo"
trap 'flatpak uninstall --user -y org.joelinux.FileGrantTest org.joelinux.GateRuntime >/dev/null 2>&1 || :; flatpak remote-delete --user "$remote" >/dev/null 2>&1 || :; rm -rf "$work"' EXIT
flatpak install --user -y "$remote" org.joelinux.FileGrantTest
flatpak run org.joelinux.FileGrantTest
