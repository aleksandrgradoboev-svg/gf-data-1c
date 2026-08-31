package tools

import (
	"strings"
	"testing"
)

func TestGateПослеОдногоОтказаТребуетПостроитель(t *testing.T) {
	g := &queryGate{}
	if ok, _ := g.checkAllowed(); !ok {
		t.Fatal("до первого отказа проверка открыта")
	}
	g.onCheckRefused("ВЫБРАТ Р.Ссылка ИЗ Документ.РеализацияТоваровУслуг КАК Р")
	ok, hint := g.checkAllowed()
	if ok {
		t.Fatal("после одного отказа проверка обязана закрыться")
	}
	if !strings.Contains(hint, "Документ.РеализацияТоваровУслуг") || !strings.Contains(hint, "query_build") {
		t.Errorf("подсказка должна нести источник и имя построителя: %s", hint)
	}
	g.onBuildCalled()
	if ok, _ := g.checkAllowed(); !ok {
		t.Fatal("вызов построителя открывает проверку снова")
	}
}

func TestGateВыполняетсяТолькоОдобренныйТекст(t *testing.T) {
	g := &queryGate{}
	built := "ВЫБРАТЬ\n\tР.Ссылка КАК Ссылка\nИЗ\n\tДокумент.РеализацияТоваровУслуг КАК Р"
	if g.isApproved(built) {
		t.Fatal("неизвестный текст не одобрен")
	}
	g.approve(built)
	if !g.isApproved("выбрать р.Ссылка как Ссылка из Документ.РеализацияТоваровУслуг как Р") {
		t.Error("тот же текст в одну строку и другим регистром обязан считаться одобренным")
	}
	if g.isApproved(built + " ГДЕ Р.Проведен") {
		t.Error("изменённый текст одобренным не считается")
	}
	g.onCheckPassed("ВЫБРАТЬ 1 КАК Один")
	if g.isApproved("ВЫБРАТЬ 1 КАК Один") {
		t.Error("прошедший проверку текст к выполнению НЕ открывается — только собранный построителем")
	}
}

func TestSourceOf(t *testing.T) {
	cases := map[string]string{
		"ВЫБРАТЬ Х.Счет ИЗ РегистрБухгалтерии.Хозрасчетный.Остатки(&Д, ) КАК Х": "РегистрБухгалтерии.Хозрасчетный.Остатки",
		"ВЫБРАТЬ 1": "",
		"выбрать а из Справочник.Номенклатура как а": "Справочник.Номенклатура",
	}
	for q, want := range cases {
		if got := sourceOf(q); got != want {
			t.Errorf("%q: получено %q, ждали %q", q, got, want)
		}
	}
}
