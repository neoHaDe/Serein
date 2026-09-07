// Собирает latest.json для апдейтера из готовых артефактов релиза.
//
//   node scripts/make-update-manifest.mjs 1.2.6 "заметки к релизу"
//
// Берёт из src-tauri/target/release/bundle установщик Windows и, если есть, AppImage;
// недостающие подписи ставит сам ключом из .tauri.
//
// Манифестов делается ДВА, и это главное, что стоит понять про этот скрипт.
//
// Требование реестра отечественного ПО - чтобы не существовало механизмов, которыми
// иностранные лица могут дистанционно ограничить работу программы. Раньше манифест лежал
// у нас, а бинарь качался с github.com: закрытый доступ к репозиторию останавливал
// обновления у всех сразу. Это и есть тот самый рычаг.
//
// Резерва на уровне отдельного файла в апдейтере Tauri нет - внутри манифеста адрес один
// на платформу. Зато `endpoints` в `tauri.conf.json` принимает СПИСОК манифестов и идёт
// по нему, пока кто-то не ответит. Поэтому манифеста два, с разными ссылками внутри:
//
//   1. `latest.json`        - ссылки на GitHub Release. Кладётся файлом в сам релиз.
//   2. `latest.mirror.json` - ссылки на nehade.xyz. Кладётся на сайт ПОД ИМЕНЕМ latest.json.
//
// GitHub стоит первым: он быстрее и не грузит домашний сервер. Если он недоступен -
// с блокировкой, без неё, неважно - апдейтер сам переходит ко второму, и обновления
// продолжают приходить. Единственной иностранной точки контроля таким образом нет.
//
// Почему AppImage подписывается здесь, а собирается на Linux-машине: приватный ключ
// должен лежать в одном месте. Подпись - это minisign поверх байтов файла, её можно
// поставить отдельно от сборки, так что ключ на сборочную VM везти незачем.

import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')

/**
 * Своя раздача - запасная.
 *
 * Вынесена в переменную окружения, чтобы домен не был зашит намертво: пригодится и для
 * проверки на другой машине, и если раздача когда-нибудь переедет.
 */
const base = (process.env.SEREIN_UPDATE_BASE ?? 'https://nehade.xyz/updates/terminal').replace(
  /\/+$/,
  ''
)


/** Куда класть файлы. Тот же каталог, откуда они потом раздаются. */
const dest = process.env.SEREIN_UPDATE_DEST ?? 'hade@192.168.0.156:/mnt/material/site/updates/terminal'
const version = process.argv[2]
const notes = process.argv[3] ?? `Serein ${version}`

if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error('нужна версия: node scripts/make-update-manifest.mjs 1.2.6 "заметки"')
  process.exit(1)
}

/** Раздача GitHub - основная. Объявлена после разбора версии: она в адресе. */
const github = `https://github.com/neoHaDe/Serein/releases/download/v${version}`

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

// Подписи считаются один раз: они по байтам файла и от адреса не зависят - потому
// один и тот же файл и можно раздавать откуда угодно.
const signatures = {}
for (const t of targets) {
  if (!existsSync(t.file)) {
    console.log(`пропускаю ${t.key}: нет ${t.file}`)
    continue
  }
  signatures[t.key] = signatureFor(t.file)
}

if (!Object.keys(signatures).length) {
  console.error('не нашёл ни одного артефакта - собери релиз перед запуском')
  process.exit(1)
}

/** Манифест с ссылками на указанную раздачу. */
function manifestFor(where) {
  const platforms = {}
  for (const t of targets) {
    if (signatures[t.key]) {
      platforms[t.key] = { signature: signatures[t.key], url: `${where}/${t.asset}` }
    }
  }
  return platforms
}

// Дата одна на оба манифеста: это один и тот же выпуск, увиденный с двух сторон.
const pub_date = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z')

const out = join(bundle, 'latest.json')
const outMirror = join(bundle, 'latest.mirror.json')
writeFileSync(
  out,
  JSON.stringify({ version, notes, pub_date, platforms: manifestFor(github) }, null, 2) + '\n',
  'utf8'
)
writeFileSync(
  outMirror,
  JSON.stringify({ version, notes, pub_date, platforms: manifestFor(base) }, null, 2) + '\n',
  'utf8'
)
console.log(`\nготово: ${out} (основная раздача, GitHub)`)
console.log(`        ${outMirror} (запасная раздача, ${base})`)
console.log(`платформы: ${Object.keys(signatures).join(', ')}`)

// Порядок важен: сначала файлы, потом манифест. Если положить манифест первым, между
// двумя командами найдётся тот, кто спросит обновление и получит ссылку на файл,
// которого ещё нет.
console.log('\n1) в релиз на GitHub - вместе с артефактами:')
console.log(`  gh release upload v${version} "${out}" --clobber`)

console.log('\n2) на свою раздачу - СНАЧАЛА файлы, ПОТОМ манифест:')
for (const t of targets) {
  if (signatures[t.key]) {
    console.log(`  scp "${t.file}" ${dest}/${t.asset}`)
  }
}
console.log(`  scp "${outMirror}" ${dest}/latest.json`)

console.log('\n3) проверить обе раздачи:')
console.log(`  curl -sI https://github.com/neoHaDe/Serein/releases/latest/download/latest.json | head -1`)
for (const t of targets) {
  if (signatures[t.key]) {
    console.log(`  curl -sI ${base}/${t.asset} | head -1`)
  }
}
