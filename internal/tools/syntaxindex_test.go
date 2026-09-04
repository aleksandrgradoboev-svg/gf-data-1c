package tools

// Проверки поиска по справке платформы.
//
// Два уровня, и они отвечают на разные вопросы. Юнит-проверки разбора и нормализации
// работают всегда: им не нужна ни справка, ни платформа. Замер по эталону требует
// собранной базы справки и файла эталона, поэтому при их отсутствии пропускается —
// пакет обязан собираться и проходить тесты на машине, где справки ещё нет.
//
// Эталон живёт вне пакета (у нас — agents-data/_shared/model-eval/syntax-truth.json)
// и подаётся через GTDATA_TRUTH: это инструмент исследователя, а не часть поставки.

import (
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestParseBracketNode(t *testing.T) {
	// Настоящая запись оглавления языка запросов.
	src := `{2,
{9,8,0,
{1,1,
{1,1,
{"#","СУММА"}
},"/SUM"}
},
{10,8,0,
{1,1,
{1,2,
{"ru","ПОДОБНО"},
{"en","LIKE"}
},"/LIKE"}
}}`
	nodes := tocNodes(src)
	if len(nodes) != 2 {
		t.Fatalf("узлов разобрано %d, ожидалось 2", len(nodes))
	}
	if nodes[0].RU != "СУММА" || nodes[0].Path != "/SUM" {
		t.Errorf("одноязычный узел разобран неверно: %+v", nodes[0])
	}
	if nodes[1].RU != "ПОДОБНО" || nodes[1].EN != "LIKE" || nodes[1].Path != "/LIKE" {
		t.Errorf("двуязычный узел разобран неверно: %+v", nodes[1])
	}
}

func TestNormKeyDropsPunctuation(t *testing.T) {
	// «ИТОГИ ... ПО» — имя темы с оформлением; многоточие не часть имени оператора.
	if normKey("ИТОГИ ... ПО") != "ИТОГИПО" {
		t.Errorf("normKey не убрал пунктуацию: %q", normKey("ИТОГИ ... ПО"))
	}
	if normKey("Литерал типа ДАТА") != "ЛИТЕРАЛТИПАДАТА" {
		t.Errorf("normKey испортил кириллицу: %q", normKey("Литерал типа ДАТА"))
	}
}

func TestSplitCamelRussian(t *testing.T) {
	got := splitCamel("ЛитералДата")
	if len(got) != 2 || got[0] != "Литерал" || got[1] != "Дата" {
		t.Errorf("склейка не разобрана: %#v", got)
	}
}

func TestQuestionKind(t *testing.T) {
	cases := map[string]string{
		"ПОДОБНО": "query",
		"ИТОГИ":   "query",
		"РегистрБухгалтерии.ОстаткиИОбороты": "table",
		"СрезПоследних":                      "table",
		"Упорядочивание результатов":         "any",
	}
	for q, want := range cases {
		if got := questionKind(q); got != want {
			t.Errorf("questionKind(%q) = %q, ожидалось %q", q, got, want)
		}
	}
}

// openTestKB открывает справку, если она собрана.
func openTestKB(t *testing.T) *sql.DB {
	t.Helper()
	path := os.Getenv("GTDATA_KB")
	if path == "" {
		path = filepath.Join("..", "..", "..", "kb", "1c-help.db")
	}
	if st, err := os.Stat(path); err != nil || st.IsDir() {
		t.Skip("справка платформы не собрана — замер пропущен")
	}
	db, err := sql.Open("sqlite", "file:"+filepath.ToSlash(path)+"?mode=ro")
	if err != nil {
		t.Skipf("база справки не открылась: %v", err)
	}
	return db
}

// TestSearchKnownPages — вопросы, на которых прежний поиск промахивался. Каждый ответ
// подтверждён страницей из справки 8.3.27.2130, а не выведен по памяти.
func TestSearchKnownPages(t *testing.T) {
	db := openTestKB(t)
	defer db.Close()

	cases := []struct {
		query string
		want  []string // достаточно попасть в любую из страниц
	}{
		{"ПОДОБНО", []string{"LIKE"}},
		{"ЕСТЬNULL", []string{"ISNULL"}},
		{"УПОРЯДОЧИТЬ ПО", []string{"ORDERBYSection"}},
		{"УПОРЯДОЧИТЬ", []string{"ORDERBYSection"}},
		{"ИТОГИ", []string{"query_totals"}},
		{"ОстаткиИОбороты", []string{"table12", "table41", "table49"}},
		{"РегистрБухгалтерии.ОстаткиИОбороты", []string{"table41", "table49"}},
		{"агрегатные функции", []string{"aggregate_functions"}},
	}
	for _, c := range cases {
		best, _ := searchHelp(db, c.query)
		if best == nil {
			t.Errorf("%q: отказ, а страница есть (ждали %v)", c.query, c.want)
			continue
		}
		ok := false
		for _, w := range c.want {
			if best.Object == w {
				ok = true
				break
			}
		}
		if !ok {
			t.Errorf("%q: отдана %s (%s), ждали одну из %v", c.query, best.Object, best.Title, c.want)
		}
	}
}

// TestSearchRefusesMissing — чего в справке нет, на то должен быть отказ, а не похожая
// страница. Подмена опаснее отказа: отказ виден, подмена — нет.
func TestSearchRefusesMissing(t *testing.T) {
	db := openTestKB(t)
	defer db.Close()

	for _, q := range []string{"ЛИМИТ", "ОстаткиДтКт", "РегистрБухгалтерии.ТаблицаРезультата"} {
		best, rest := searchHelp(db, q)
		if best != nil {
			t.Errorf("%q: выдана страница %s (%s), ожидался отказ", q, best.Object, best.Title)
		}
		_ = rest // список кандидатов к отказу проверяется в тесте инструмента
	}
}

// TestSearchAgainstTruth — полный замер по эталону, если он подан через GTDATA_TRUTH.
// Порог тот же, что в приёмке задачи: мимо не больше 15%.
func TestSearchAgainstTruth(t *testing.T) {
	truthPath := os.Getenv("GTDATA_TRUTH")
	if truthPath == "" {
		t.Skip("эталон не подан (GTDATA_TRUTH) — полный замер пропущен")
	}
	raw, err := os.ReadFile(truthPath)
	if err != nil {
		t.Skipf("эталон не прочитан: %v", err)
	}
	var truth struct {
		Themes map[string]struct {
			Expect []string `json:"expect"`
			Refuse bool     `json:"refuse"`
		} `json:"themes"`
	}
	if err := json.Unmarshal(raw, &truth); err != nil {
		t.Fatalf("эталон не разобран: %v", err)
	}

	db := openTestKB(t)
	defer db.Close()

	var hit, rightRefuse, swap, falseRefuse, invention int
	for theme, spec := range truth.Themes {
		best, _ := searchHelp(db, theme)
		switch {
		case spec.Refuse && best == nil:
			rightRefuse++
		case spec.Refuse:
			invention++
			t.Logf("выдумка: %q -> %s (%s)", theme, best.Object, best.Title)
		case best == nil:
			falseRefuse++
			t.Logf("ложный отказ: %q", theme)
		default:
			ok := false
			for _, w := range spec.Expect {
				if best.Object == w {
					ok = true
					break
				}
			}
			if ok {
				hit++
			} else {
				swap++
				t.Logf("подмена: %q -> %s (%s)", theme, best.Object, best.Title)
			}
		}
	}
	total := len(truth.Themes)
	good := hit + rightRefuse
	miss := total - good
	t.Logf("тем %d | попадание %d | верный отказ %d | подмена %d | ложный отказ %d | выдумка %d",
		total, hit, rightRefuse, swap, falseRefuse, invention)
	t.Logf("ВЕРНО %d из %d (%d%%) | МИМО %d (%d%%)", good, total, good*100/total, miss, miss*100/total)
	if miss*100/total > 15 {
		t.Errorf("мимо %d%% при пороге приёмки 15%%", miss*100/total)
	}
}
