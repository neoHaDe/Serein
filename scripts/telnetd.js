// Проверочный telnet-сервер. Ведёт себя как сетевая железка: сам предлагает опции,
// спрашивает тип терминала, ждёт размер окна и пишет в журнал всё, что получил.
// Нужен, чтобы проверить клиента без живого оборудования.
const net = require('net')
const fs = require('fs')

const PORT = Number(process.argv[2] || 2323)
const LOG = process.argv[3] || 'telnetd.log'

const IAC = 255, DONT = 254, DO = 253, WONT = 252, WILL = 251, SB = 250, SE = 240
const OPT = { 0: 'BINARY', 1: 'ECHO', 3: 'SGA', 24: 'TTYPE', 31: 'NAWS', 32: 'TSPEED', 34: 'LINEMODE' }
const VERB = { 251: 'WILL', 252: 'WONT', 253: 'DO', 254: 'DONT' }

const out = fs.createWriteStream(LOG, { flags: 'w' })
const log = (s) => {
  out.write(new Date().toISOString().slice(11, 23) + '  ' + s + '\n')
}

const server = net.createServer((sock) => {
  log(`=== подключение ${sock.remoteAddress}:${sock.remotePort}`)
  let state = 'data'
  let verb = 0
  let sbOpt = 0
  let sb = []
  let line = ''

  // Как настоящая железка: сразу навязываем свои условия.
  sock.write(Buffer.from([IAC, WILL, 1, IAC, WILL, 3, IAC, DO, 24, IAC, DO, 31, IAC, DO, 32]))
  log('послал: WILL ECHO, WILL SGA, DO TTYPE, DO NAWS, DO TSPEED')
  sock.write('Test switch console\r\nlogin: ')

  sock.on('data', (buf) => {
    for (const b of buf) {
      switch (state) {
        case 'data':
          if (b === IAC) { state = 'iac'; break }
          if (b === 13) { // CR — смотрим, что придёт следом
            log(`строка: ${JSON.stringify(line)}`)
            sock.write('\r\n> ')
            line = ''
            state = 'cr'
            break
          }
          line += String.fromCharCode(b)
          sock.write(Buffer.from([b])) // эхо: мы объявили WILL ECHO
          break
        case 'cr':
          // Показываем, чем клиент завершает строку: LF, NUL или сразу данные.
          log(`после CR пришёл байт ${b} (${b === 10 ? 'LF' : b === 0 ? 'NUL' : 'данные'})`)
          state = 'data'
          if (b !== 10 && b !== 0) { line += String.fromCharCode(b); sock.write(Buffer.from([b])) }
          break
        case 'iac':
          if (b === IAC) { line += '\xff'; state = 'data'; break }
          if (b === SB) { state = 'sbopt'; break }
          if (VERB[b]) { verb = b; state = 'verb'; break }
          log(`команда ${b}`)
          state = 'data'
          break
        case 'verb':
          log(`получил: ${VERB[verb]} ${OPT[b] || b}`)
          // Согласился сообщать тип терминала — сразу спрашиваем какой.
          if (verb === WILL && b === 24) {
            sock.write(Buffer.from([IAC, SB, 24, 1, IAC, SE]))
            log('послал: SB TTYPE SEND')
          }
          // На предложение включить то, чего мы не просили, отвечаем отказом.
          if (verb === WILL && ![0, 3, 24, 31].includes(b)) sock.write(Buffer.from([IAC, DONT, b]))
          state = 'data'
          break
        case 'sbopt':
          sbOpt = b; sb = []; state = 'sb'
          break
        case 'sb':
          if (b === IAC) { state = 'sbiac'; break }
          sb.push(b)
          break
        case 'sbiac':
          if (b === SE) {
            if (sbOpt === 24) log(`тип терминала: ${Buffer.from(sb.slice(1)).toString()}`)
            else if (sbOpt === 31) log(`размер окна: ${(sb[0] << 8) | sb[1]} x ${(sb[2] << 8) | sb[3]}`)
            else log(`подкоманда ${OPT[sbOpt] || sbOpt}: ${sb.join(',')}`)
            state = 'data'
          } else { sb.push(b); state = 'sb' }
          break
      }
    }
  })

  sock.on('close', () => log('=== отключение'))
  sock.on('error', (e) => log('ошибка сокета: ' + e.message))
})

server.listen(PORT, '127.0.0.1', () => log(`слушаю 127.0.0.1:${PORT}`))
