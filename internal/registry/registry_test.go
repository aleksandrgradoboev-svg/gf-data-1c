package registry_test

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/greentech/gt-data-1c/internal/refusal"
	"github.com/greentech/gt-data-1c/internal/registry"
)

func load(t *testing.T) *registry.Registry {
	t.Helper()
	reg, err := registry.Load(filepath.Join(t.TempDir(), "bases.json"))
	if err != nil {
		t.Fatalf("реестр не прочитан: %v", err)
	}
	return reg
}

func add(t *testing.T, reg *registry.Registry, names ...string) {
	t.Helper()
	for _, n := range names {
		if err := reg.Add(registry.Base{Name: n, URL: "http://127.0.0.1:1/" + n}); err != nil {
			t.Fatalf("база %q не добавлена: %v", n, err)
		}
	}
}

func refusalOf(t *testing.T, err error) *refusal.Error {
	t.Helper()
	var ref *refusal.Error
	if !errors.As(err, &ref) {
		t.Fatalf("ожидался отказ, получено: %v", err)
	}
	return ref
}

// TestResolveБезИмени — пустое имя всегда отказ, сколько бы баз ни было.
// Три случая проверяются вместе намеренно: разойдись они, вернулось бы умолчание
// «в частном случае», а такое умолчание — то же самое умолчание.
func TestResolveБезИмени(t *testing.T) {
	for _, tc := range []struct {
		имя  string
		базы []string
	}{
		{"реестр пуст", nil},
		{"одна база", []string{"ut11"}},
		{"несколько баз", []string{"ut11", "bu3"}},
	} {
		t.Run(tc.имя, func(t *testing.T) {
			reg := load(t)
			add(t, reg, tc.базы...)
			_, err := reg.Resolve("")
			ref := refusalOf(t, err)
			if ref.Kind != refusal.BadRequest {
				t.Errorf("вид отказа: %v, ожидался BadRequest", ref.Kind)
			}
			if ref.What != "база не названа" {
				t.Errorf("причина: %q", ref.What)
			}
			if !strings.Contains(strings.Join(ref.Hints, " "), "base") {
				t.Errorf("подсказка должна называть параметр base: %v", ref.Hints)
			}
		})
	}
}

// TestResolveНезнакомойБазы — отказ обязан перечислить известные, иначе вызывающий
// решит, что база пуста.
func TestResolveНезнакомойБазы(t *testing.T) {
	reg := load(t)
	add(t, reg, "ut11", "bu3")
	_, err := reg.Resolve("нетакой")
	ref := refusalOf(t, err)
	if ref.Kind != refusal.UnknownBase {
		t.Errorf("вид отказа: %v, ожидался UnknownBase", ref.Kind)
	}
	if !strings.Contains(ref.Why, "ut11") || !strings.Contains(ref.Why, "bu3") {
		t.Errorf("отказ должен перечислить известные базы, получено: %q", ref.Why)
	}
}

// TestResolveПоИмени — названная база находится, регистр букв не важен.
func TestResolveПоИмени(t *testing.T) {
	reg := load(t)
	add(t, reg, "UT11")
	base, err := reg.Resolve("ut11")
	if err != nil {
		t.Fatalf("названная база не разрешилась: %v", err)
	}
	if base.Name != "UT11" {
		t.Errorf("вернулась база %q", base.Name)
	}
}

// TestУмолчаниеИзФайлаНеОживает — реестр, записанный прежней версией, несёт ключ
// "default". Он обязан остаться мёртвым текстом, а не восстановить механизм.
func TestУмолчаниеИзФайлаНеОживает(t *testing.T) {
	path := filepath.Join(t.TempDir(), "bases.json")
	старый := `{"default":"ut11","bases":[{"name":"ut11","url":"http://127.0.0.1:1/ut11"}]}`
	if err := writeFile(path, старый); err != nil {
		t.Fatalf("файл не записан: %v", err)
	}
	reg, err := registry.Load(path)
	if err != nil {
		t.Fatalf("реестр не прочитан: %v", err)
	}
	if _, err := reg.Resolve(""); err == nil {
		t.Error("ключ default из старого файла воскресил базу по умолчанию")
	}
}

func writeFile(path, content string) error {
	return os.WriteFile(path, []byte(content), 0o600)
}

// Update правит названные поля и НЕ трогает остальные. Главное здесь — пароль:
// наверх он не отдаётся, значит UI прислать его обратно не может, и правка адреса
// не имеет права его стереть. Через Add этот сценарий ломался молча.
func TestUpdateСоседниеПоляНеТрогает(t *testing.T) {
	path := filepath.Join(t.TempDir(), "bases.json")
	reg, err := registry.Load(path)
	if err != nil {
		t.Fatalf("реестр не прочитан: %v", err)
	}
	if err := reg.Add(registry.Base{
		Name: "ut11", Title: "УТ 11", URL: "http://old/hs/gt-data",
		User: "Админ", Password: "тайна", Auth: "basic",
	}); err != nil {
		t.Fatalf("Add: %v", err)
	}

	base, changed, err := reg.Update(registry.Patch{Name: "ut11", URL: "http://new/hs/gt-data"})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if len(changed) != 1 || changed[0] != "адрес" {
		t.Fatalf("изменения: %v, ожидался только адрес", changed)
	}
	if base.URL != "http://new/hs/gt-data" {
		t.Errorf("адрес не изменился: %q", base.URL)
	}
	if base.User != "Админ" || base.Title != "УТ 11" || base.Auth != "basic" {
		t.Errorf("затёрты соседние поля: %+v", base)
	}
	pass, err := base.Secret()
	if err != nil {
		t.Fatalf("Secret: %v", err)
	}
	if pass != "тайна" {
		t.Errorf("пароль затёрт правкой адреса: %q", pass)
	}

	// Правка без Save — это правка, которой не было.
	again, err := registry.Load(path)
	if err != nil {
		t.Fatalf("повторное чтение: %v", err)
	}
	got, err := again.Resolve("ut11")
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	if got.URL != "http://new/hs/gt-data" {
		t.Errorf("на диске остался старый адрес: %q", got.URL)
	}
}

func TestUpdateПустойПатчНичегоНеМеняет(t *testing.T) {
	reg := load(t)
	if err := reg.Add(registry.Base{Name: "buh", URL: "http://x/hs/gt-data", User: "u"}); err != nil {
		t.Fatalf("Add: %v", err)
	}

	_, changed, err := reg.Update(registry.Patch{Name: "buh"})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if len(changed) != 0 {
		t.Errorf("пустой патч дал изменения: %v", changed)
	}
}

func TestUpdateНазваниеСнимаетсяТолькоЯвно(t *testing.T) {
	reg := load(t)
	if err := reg.Add(registry.Base{Name: "buh", Title: "Бухгалтерия", URL: "http://x/hs/gt-data"}); err != nil {
		t.Fatalf("Add: %v", err)
	}

	// Пустой Title — «не менять».
	base, _, err := reg.Update(registry.Patch{Name: "buh", Title: ""})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if base.Title != "Бухгалтерия" {
		t.Errorf("пустой title снял название: %q", base.Title)
	}

	// Явный признак — снять.
	base, changed, err := reg.Update(registry.Patch{Name: "buh", ClearTitle: true})
	if err != nil {
		t.Fatalf("Update с ClearTitle: %v", err)
	}
	if base.Title != "" {
		t.Errorf("ClearTitle не снял название: %q", base.Title)
	}
	if len(changed) == 0 {
		t.Error("снятие названия не попало в перечень изменений")
	}
}

func TestUpdateНезнакомаяБазаОтказ(t *testing.T) {
	reg := load(t)
	if err := reg.Add(registry.Base{Name: "buh", URL: "http://x/hs/gt-data"}); err != nil {
		t.Fatalf("Add: %v", err)
	}

	if _, _, err := reg.Update(registry.Patch{Name: "нет-такой", URL: "http://y"}); err == nil {
		t.Fatal("правка несуществующей базы прошла без отказа")
	}
}
