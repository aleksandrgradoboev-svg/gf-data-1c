package installer

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Поиск платформы обязан смотреть шире двух путей на диске C: платформу ставят
// на другой диск, 64-битная живёт в 1cv8x64, каталог задают руками. Поймано
// вопросом о машине тестера: там платформа лежала не там, где ожидал установщик.
func TestPlatformRootsПокрываетНестандартныеМеста(t *testing.T) {
	t.Setenv("GT_DATA_1C_PLATFORM", `D:\своя-1с`)
	roots := platformRoots()

	if len(roots) == 0 {
		t.Fatal("корней поиска нет вовсе")
	}
	if roots[0] != `D:\своя-1с` {
		t.Errorf("явно заданный каталог обязан идти первым, получено: %q", roots[0])
	}

	joined := strings.Join(roots, "|")
	for _, want := range []string{"1cv8x64", "1cv8"} {
		if !strings.Contains(joined, want) {
			t.Errorf("каталог %q не попал в перечень: %v", want, roots)
		}
	}
}

func TestPlatformRootsБезПовторов(t *testing.T) {
	t.Setenv("GT_DATA_1C_PLATFORM", "")
	roots := platformRoots()
	seen := map[string]bool{}
	for _, r := range roots {
		low := strings.ToLower(r)
		if seen[low] {
			t.Errorf("корень повторяется: %s (перечень: %v)", r, roots)
		}
		seen[low] = true
	}
}

// Отказ обязан называть, ГДЕ искали: без этого человек на чужой машине не знает,
// добавить ли путь флагом или платформы нет вовсе. Проверяется на заведомо
// пустых корнях, а не на машине разработчика: там платформа стоит в стандартном
// месте, отказа не случается, и тест молчал бы именно там, где он нужен.
func TestОтказНазываетМестаПоиска(t *testing.T) {
	empty := t.TempDir()
	_, err := resolvePlatformIn("", []string{filepath.Join(empty, "нет-платформы")})
	if err == nil {
		t.Fatal("на пустом каталоге отказа не случилось")
	}
	msg := err.Error()
	for _, want := range []string{"Искали в:", "-platform", "GT_DATA_1C_PLATFORM", "нет-платформы"} {
		if !strings.Contains(msg, want) {
			t.Errorf("в отказе нет %q: %s", want, msg)
		}
	}
}

// Явно указанный путь до самого exe принимается как есть — на машине, где
// каталоги названы не по-нашему, это единственный надёжный путь.
func TestЯвныйПутьКФайлуПринимается(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "1cv8.exe")
	if err := os.WriteFile(exe, []byte("не настоящий"), 0o600); err != nil {
		t.Fatalf("файл-заглушка не создан: %v", err)
	}

	got, err := resolvePlatform(exe)
	if err != nil {
		t.Fatalf("явный путь к файлу отвергнут: %v", err)
	}
	if got != exe {
		t.Errorf("вернулся другой путь: %q", got)
	}

	// И каталог, в котором лежит 1cv8.exe, тоже принимается.
	got, err = resolvePlatform(dir)
	if err != nil {
		t.Fatalf("каталог с 1cv8.exe отвергнут: %v", err)
	}
	if got != exe {
		t.Errorf("из каталога собран неверный путь: %q", got)
	}
}

// Расширение кладётся в бинарь отдельным шагом сборки, и каталог для него — артефакт,
// которого в репозитории нет. Проверяется здесь то, что бинарь без расширения об этом
// ГОВОРИТ: прежде такая сборка доходила до конфигуратора и получала отказ про
// «принадлежность основного объекта конфигурации», отправляющий искать причину не туда.
func TestРасширениеВстроеноВЭтуСборку(t *testing.T) {
	if !extensionBuilt() {
		t.Skip("расширение не собрано — это законная сборка, проверять нечего")
	}

	// Собранный бинарь обязан нести главный файл выгрузки: по нему и определяется,
	// что расширение на месте.
	if _, err := extensionFS.ReadFile("extension/Configuration.xml"); err != nil {
		t.Fatalf("расширение объявлено встроенным, но Configuration.xml не читается: %v", err)
	}
}

// Отказ несобранной сборки называет способ починки, а не только факт.
func TestОтказБезРасширенияНазываетПочинку(t *testing.T) {
	text := ErrExtensionNotBuilt.Error()
	for _, обязательное := range []string{"build-extension.ps1", "go build", "релиз"} {
		if !strings.Contains(strings.ToLower(text), strings.ToLower(обязательное)) {
			t.Errorf("в отказе нет упоминания %q; отказ: %s", обязательное, text)
		}
	}
}
