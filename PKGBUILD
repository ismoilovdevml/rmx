pkgname=rmx
pkgver=1.0.0
pkgrel=1
pkgdesc="A program written in Rust for deleting large and very large files"
arch=('x86_64')
url="https://github.com/ismoilovdevml/rmx"
license=('MIT')
depends=('gcc-libs')
makedepends=('rust')
source=("${pkgname}-${pkgver}.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP') # Replace with the actual checksum

build() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname-$pkgver"
  install -Dm755 target/release/$pkgname "$pkgdir/usr/bin/$pkgname"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  
  # Add completion for the rmx command
  install -Dm644 completions/rmx.bash-completion "$pkgdir/usr/share/bash-completion/completions/rmx"
}

