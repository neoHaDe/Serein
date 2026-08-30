// Собирает latest.json для апдейтера из готовых артефактов релиза.
//
//   node scripts/make-update-manifest.mjs 1.2.6 "заметки к релизу"
//
// Берёт из src-tauri/target/release/bundle установщик Windows и, если есть, AppImage;
// недостающие подписи ставит сам ключом из .tauri. Ссылки ведут на GitHub Release,
// поэтому на сайт кладётся только этот файл — бинарники туда заливать не нужно.
//
// Почему AppImage подписывается здесь, а собирается на Linux-машине: приватный ключ
// должен лежать в одном месте. Подпись — это minisign поверх байтов файла, её можно
// поставить отдельно от сборки, так что ключ на сборочную VM везти незачем.

import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const version = process.argv[2]
const notes = process.argv[3] ?? `Serein ${version}`

if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error('нужна версия: node scripts/make-update-manifest.mjs 1.2.6 "заметки"')
  process.exit(1)
}

const bundle = join(root, 'src-tauri', 'target', 'release', 'bundle')
const keyPath = join(root, '.tauri', 'terminal.key')
const passPath = join(root, '.tauri', 'password.txt')

/** Подписать файл, если подписи ещё нет. Ключ и пароль не печатаем никуда. */
function signatureFor(file) {
  const sig = `${file}.sig`
  if (!existsSync(sig)) {
    if (!existsSync(keyPath)) {
      throw new Error(`нет подписи ${sig} и нет ключа ${keyPath}`)
    }
    const args = ['run', 'tauri', '--', 'signer', 'sign', '--private-key-path', keyPath, file]
    if (existsSync(passPath)) {
      args.splice(-1, 0, '--password', readFileSync(passPath, 'utf8').trim())
    }
    console.log(`подписываю ${file}`)
    execFileSync('npm', args, { cwd: root, stdio: 'inherit', shell: process.platform === 'win32' })
  }
  return readFileSync(sig, 'utf8').trim()
}

const targets = [
  {
    key: 'windows-x86_64',
    file: join(bundle, 'nsis', `Serein_${version}_x64-setup.exe`),
    asset: `Serein_${version}_x64-setup.exe`
  },
  {
    key: 'linux-x86_64',
    // Только AppImage: .deb обновляется менеджером пакетов, приложение туда не пишет.
    file: join(bundle, 'appimage', `Serein_${version}_amd64.AppImage`),
    asset: `Serein_${version}_amd64.AppImage`
  }
]

const platforms = {}
for (const t of targets) {
  if (!existsSync(t.file)) {
    console.log(`пропускаю ${t.key}: нет ${t.file}`)
    continue
  }
  platforms[t.key] = {
    signature: signatureFor(t.file),
    url: `https://github.com/neoHaDe/Serein/releases/download/v${version}/${t.asset}`
  }
}

if (!Object.keys(platforms).length) {
  console.error('не нашёл ни одного артефакта — собери релиз перед запуском')
  process.exit(1)
}

const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
  platforms
}

const out = join(bundle, 'latest.json')
writeFileSync(out, JSON.stringify(manifest, null, 2) + '\n', 'utf8')
console.log(`\nготово: ${out}`)
console.log(`платформы: ${Object.keys(platforms).join(', ')}`)
console.log('\nвыложить на сайт:')
console.log(
  `  scp "${out}" hade@192.168.0.156:/mnt/material/site/updates/terminal/latest.json`
)
