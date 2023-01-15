# Maintainer: Ismoilovdev <ismoilovdev@gmail.com>
pkgname=rmx
pkgver=0.1.0
pkgrel=1
pkgdesc='A program written in the Rust programming language for deleting large and very large files'
url='https://github.com/ismoilovdevml/rmx'
source=("$pkgname-$pkgver.tar.gz::https://github.com/ismoilovdevml/rmx/archive/v$pkgver.tar.gz")
backup=("etc/rmx.conf")
arch=('i686' 'pentium4' 'x86_64' 'arm' 'armv7h' 'armv6h' 'aarch64')
license=('GPL3')
makedepends=('cargo')
depends=('git' 'pacman')
optdepends=('asp: downloading repo pkgbuilds' 'bat: colored pkgbuild printing' 'devtools: build in chroot')
sha256sums=('7fb65a143b226bca1d54d58b544a4c484f04748bd38012feaef2e76226575e6d')

build () {
  cd "$srcdir/$pkgname-$pkgver"

  if pacman -T pacman-git > /dev/null; then
    _features+="git,"
  fi

  if [[ $CARCH != x86_64 ]]; then
    export CARGO_PROFILE_RELEASE_LTO=off
  fi

  cargo build --locked --features "${_features:-}" --release --target-dir target
  ./scripts/mkmo locale/
}

package() {
  cd "$srcdir/$pkgname-$pkgver"

  install -Dm755 target/release/rmx "${pkgdir}/usr/bin/rmx"
  install -Dm644 rmx.conf "${pkgdir}/etc/rmx.conf"

  install -Dm644 man/rmx.8 "$pkgdir/usr/share/man/man8/rmx.8"
  install -Dm644 man/rmx.conf.5 "$pkgdir/usr/share/man/man5/rmx.conf.5"

  install -Dm644 completions/bash "${pkgdir}/usr/share/bash-completion/completions/rmx.bash"
  install -Dm644 completions/fish "${pkgdir}/usr/share/fish/vendor_completions.d/rmx.fish"
  install -Dm644 completions/zsh "${pkgdir}/usr/share/zsh/site-functions/_rmx"

  install -d "$pkgdir/usr/share/"
  cp -r locale "$pkgdir/usr/share/"
}