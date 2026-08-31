// Ставит версию сразу везде, где она нужна.
//
// Мест два: package.json и src-tauri/Cargo.toml. Третье — tauri.conf.json — теперь
// ссылается на package.json и правки не требует. Раньше все три правились руками, и
// разъехаться они могли молча: установщик, «о программе» и апдейтер начинали называть
// разные версии, а замечалось это уже после публикации. Сборка теперь такое расхождение
// не пропустит (`build.rs`), но чинить его лучше до, а не после.
//
// Запуск: node scripts/set-version.mjs 1.2.7
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const version = process.argv[2]

if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) {
  console.error('Использование: node scripts/set-version.mjs 1.2.7')
  process.exit(1)
}

// package.json — правим только поле верхнего уровня, не трогая версии зависимостей.
const pkgPath = join(root, 'package.json')
const pkg = readFileSync(pkgPath, 'utf8')
const pkgField = /("version"\s*:\s*")[^"]+(")/
// Проверяем наличие поля, а не факт изменения: если версия уже нужная, текст совпадёт,
// и «ничего не поменялось» — это успех, а не ошибка.
if (!pkgField.test(pkg)) {
  console.error('не нашёл поле version в package.json')
  process.exit(1)
}
writeFileSync(pkgPath, pkg.replace(pkgField, `$1${version}$2`))

// Cargo.toml — только version в секции [package], которая идёт первой.
const cargoPath = join(root, 'src-tauri', 'Cargo.toml')
const cargo = readFileSync(cargoPath, 'utf8')
const cargoField = /^version = "[^"]+"$/m
if (!cargoField.test(cargo)) {
  console.error('не нашёл version в src-tauri/Cargo.toml')
  process.exit(1)
}
writeFileSync(cargoPath, cargo.replace(cargoField, `version = "${version}"`))

console.log(`версия ${version}: package.json, src-tauri/Cargo.toml`)
console.log('tauri.conf.json трогать не надо — он ссылается на package.json')
