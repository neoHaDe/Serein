// Набор правил намеренно узкий.
//
// Типы проверяет `tsc` со `strict`, `noUnusedLocals` и `noUnusedParameters` - дублировать
// его здесь незачем, иначе линтер превращается во второй компилятор с чуть другим мнением.
// Линтеру оставлено то, чего компилятор не видит: промахи с хуками React (вызов из условия,
// забытая зависимость) и несколько ловушек, которые тихо меняют поведение.
import js from '@eslint/js'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'

export default tseslint.config(
  { ignores: ['dist/**', 'src-tauri/**', 'node_modules/**', 'docs/**'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      // Неиспользованное ловит `tsc`; здесь правило выключено, чтобы не спорить с ним
      // о подчёркиваниях и о параметрах, оставленных для читаемости подписи.
      '@typescript-eslint/no-unused-vars': 'off',
      // `==` с null - привычная и намеренная проверка «null или undefined».
      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'no-var': 'error',
      'prefer-const': 'error'
    }
  }
)
