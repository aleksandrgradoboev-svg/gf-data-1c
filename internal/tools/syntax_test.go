package tools

import (
	"path/filepath"
	"testing"
)

// Порядок поиска базы справки — это правило, а не деталь реализации: справка платформы и справка
// типовых конфигураций носили одно имя файла, лежали в разных каталогах и разошлись, чего никто
// не заметил, пока инструмент молча отказывал. Тест держит порядок явным.
func TestKbCandidatesПорядокИмён(t *testing.T) {
	got := kbCandidates("", `C:\пакет`, `C:\работа`)
	want := []string{
		filepath.Join(`C:\пакет`, "kb", "1c-platform-help.db"),
		filepath.Join(`C:\пакет`, "kb", "1c-help.db"),
		filepath.Join(`C:\работа`, "kb", "1c-platform-help.db"),
		filepath.Join(`C:\работа`, "kb", "1c-help.db"),
	}
	if len(got) != len(want) {
		t.Fatalf("кандидатов %d, ожидалось %d: %v", len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("кандидат %d: получено %q, ожидалось %q", i, got[i], want[i])
		}
	}
}

// Переменная окружения важнее любого файла рядом с пакетом: ею справку подменяют осознанно.
func TestKbCandidatesПеременнаяПервая(t *testing.T) {
	got := kbCandidates(`D:\своя.db`, `C:\пакет`, "")
	if len(got) == 0 || got[0] != `D:\своя.db` {
		t.Fatalf("GTDATA_KB должен идти первым, получено: %v", got)
	}
}

// Пустой каталог не превращается в путь от корня: иначе поиск ушёл бы в \kb\… на текущем диске.
func TestKbCandidatesПустыеКаталогиПропускаются(t *testing.T) {
	if got := kbCandidates("", "", ""); len(got) != 0 {
		t.Fatalf("без каталогов и переменной кандидатов быть не должно, получено: %v", got)
	}
}
