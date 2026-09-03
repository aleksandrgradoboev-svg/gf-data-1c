package server_test

import (
	"context"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/greentech/gt-data-1c/internal/server"
)

// connect поднимает сервер на временном реестре и возвращает клиентскую сессию.
func connect(t *testing.T) (*mcp.ClientSession, context.Context) {
	t.Helper()
	ctx := context.Background()
	regPath := filepath.Join(t.TempDir(), "bases.json")

	srv := server.New(server.Options{RegistryPath: regPath, Timeout: 3 * time.Second, AllowRawQuery: true})
	clientTransport, serverTransport := mcp.NewInMemoryTransports()
	if _, err := srv.Connect(ctx, serverTransport, nil); err != nil {
		t.Fatalf("сервер не поднялся: %v", err)
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "test"}, nil)
	cs, err := client.Connect(ctx, clientTransport, nil)
	if err != nil {
		t.Fatalf("клиент не подключился: %v", err)
	}
	t.Cleanup(func() { cs.Close() })
	return cs, ctx
}

func call(t *testing.T, cs *mcp.ClientSession, ctx context.Context, name string, args map[string]any) (string, bool) {
	t.Helper()
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: name, Arguments: args})
	if err != nil {
		t.Fatalf("вызов %s сорвался: %v", name, err)
	}
	var b strings.Builder
	for _, c := range res.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			b.WriteString(tc.Text)
		}
	}
	return b.String(), res.IsError
}

func TestИнструментыОбъявлены(t *testing.T) {
	cs, ctx := connect(t)
	res, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("список инструментов не получен: %v", err)
	}
	got := map[string]bool{}
	for _, tool := range res.Tools {
		got[tool.Name] = true
	}
	for _, want := range []string{"bases", "probe"} {
		if !got[want] {
			t.Errorf("инструмент %q не объявлен", want)
		}
	}
}

func TestПустойРеестрГоворитЧтоОнПуст(t *testing.T) {
	cs, ctx := connect(t)
	out, isErr := call(t, cs, ctx, "bases", map[string]any{"action": "list"})
	if isErr {
		t.Fatalf("list не должен быть отказом: %s", out)
	}
	if !strings.Contains(out, "Реестр баз пуст") {
		t.Errorf("пустой реестр должен называть себя пустым, получено: %s", out)
	}
}

func TestНезнакомаяБазаДаётОтказСПеречнем(t *testing.T) {
	cs, ctx := connect(t)
	call(t, cs, ctx, "bases", map[string]any{
		"action": "add", "name": "ut11", "url": "http://127.0.0.1:1/data",
	})
	out, isErr := call(t, cs, ctx, "probe", map[string]any{"base": "нетакой"})
	if !isErr {
		t.Fatalf("незнакомая база обязана быть отказом, получено: %s", out)
	}
	if !strings.Contains(out, "ОТКАЗ") || !strings.Contains(out, "ut11") {
		t.Errorf("отказ должен называть известные базы, получено: %s", out)
	}
}

func TestПробаРазличаетВидыОтказа(t *testing.T) {
	cs, ctx := connect(t)

	// Живая база: отвечает версией так же, как настоящее расширение — с признаком ok.
	live := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/version") {
			w.Write([]byte(`{"ok":true,"продукт":"gt-data-1c","расширение":"0.1.0","платформа":"8.3.27.2130"}`))
			return
		}
		http.NotFound(w, r)
	}))
	defer live.Close()

	// Расширения нет: 1С отвечает 404 на неизвестный маршрут — тело без HTML.
	noExt := httptest.NewServer(http.HandlerFunc(http.NotFound))
	defer noExt.Close()

	// Публикации нет: 404 рисует сам веб-сервер своей HTML-страницей, до 1С
	// запрос не дошёл. Замерено на живом Apache 03.09.2026 — именно этим
	// два 404 и различаются, а не длиной ответа.
	noPub := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=iso-8859-1")
		w.WriteHeader(http.StatusNotFound)
		w.Write([]byte("<html><head><title>404 Not Found</title></head><body><h1>Not Found</h1></body></html>"))
	}))
	defer noPub.Close()

	// Прав нет: 401.
	noAuth := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
	}))
	defer noAuth.Close()

	for _, b := range []struct{ name, url string }{
		{"alive", live.URL},
		{"noext", noExt.URL},
		{"nopub", noPub.URL},
		{"noauth", noAuth.URL},
		{"nosrv", "http://127.0.0.1:1/data"}, // никто не слушает
	} {
		out, isErr := call(t, cs, ctx, "bases", map[string]any{
			"action": "add", "name": b.name, "url": b.url,
		})
		if isErr {
			t.Fatalf("база %s не добавлена: %s", b.name, out)
		}
	}

	out, isErr := call(t, cs, ctx, "probe", nil)
	if isErr {
		t.Fatalf("проба не должна быть отказом: %s", out)
	}
	checks := []struct{ base, want string }{
		{"alive", "жив"},
		{"noext", "расширение не отвечает"},
		{"nopub", "базы нет по этому адресу"},
		{"noauth", "отказ прав"},
		{"nosrv", "веб-сервер не отвечает"},
	}
	for _, c := range checks {
		line := lineFor(out, c.base)
		if !strings.Contains(line, c.want) {
			t.Errorf("база %s: ожидалось %q, строка отчёта: %q", c.base, c.want, line)
		}
	}
	if !strings.Contains(out, "Живых каналов: 1 из 5") {
		t.Errorf("итоговый счётчик неверен: %s", out)
	}
}

func lineFor(out, base string) string {
	for _, line := range strings.Split(out, "\n") {
		if strings.Contains(line, base) {
			return line
		}
	}
	return ""
}
