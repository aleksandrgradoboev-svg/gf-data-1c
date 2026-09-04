package server_test

import (
	"context"
	"net/http"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/aleksandrgradoboev-svg/gf-data-1c/internal/server"
)

// Сетевой режим существует ради одного: несколько сессий агента работают с базами
// одновременно через один процесс. Это и проверяется — параллельность, а не факт ответа.

func serveTest(t *testing.T, token string) string {
	t.Helper()
	regPath := filepath.Join(t.TempDir(), "bases.json")
	opts := server.Options{RegistryPath: regPath, Timeout: 30 * time.Second}

	// Порт занимает сама ОС: фиксированный номер в тестах рано или поздно окажется занят.
	addrCh := make(chan string, 1)
	go func() {
		_ = server.ServeHTTPListener(opts, server.HTTPOptions{Addr: "127.0.0.1:0", Token: token}, addrCh)
	}()

	select {
	case addr := <-addrCh:
		return "http://" + addr + "/mcp"
	case <-time.After(5 * time.Second):
		t.Fatal("сетевой сервер не поднялся за пять секунд")
		return ""
	}
}

func connectHTTP(t *testing.T, endpoint, token string) *mcp.ClientSession {
	t.Helper()
	transport := &mcp.StreamableClientTransport{Endpoint: endpoint}
	if token != "" {
		transport.HTTPClient = &http.Client{Transport: bearer{token: token, base: http.DefaultTransport}}
	}
	client := mcp.NewClient(&mcp.Implementation{Name: "http-test"}, nil)
	cs, err := client.Connect(context.Background(), transport, nil)
	if err != nil {
		t.Fatalf("клиент не подключился: %v", err)
	}
	t.Cleanup(func() { cs.Close() })
	return cs
}

type bearer struct {
	token string
	base  http.RoundTripper
}

func (b bearer) RoundTrip(req *http.Request) (*http.Response, error) {
	req = req.Clone(req.Context())
	req.Header.Set("Authorization", "Bearer "+b.token)
	return b.base.RoundTrip(req)
}

func TestСетевойРежимДержитНесколькоСессий(t *testing.T) {
	endpoint := serveTest(t, "")
	ctx := context.Background()

	const сессий = 4
	var wg sync.WaitGroup
	ошибки := make(chan string, сессий)

	for i := 0; i < сессий; i++ {
		wg.Add(1)
		go func(n int) {
			defer wg.Done()
			cs := connectHTTP(t, endpoint, "")

			// Каждая сессия ведёт свою переписку: список инструментов и вызов.
			tools, err := cs.ListTools(ctx, nil)
			if err != nil {
				ошибки <- "сессия не получила список инструментов: " + err.Error()
				return
			}
			if len(tools.Tools) == 0 {
				ошибки <- "сессия получила пустой список инструментов"
				return
			}

			res, err := cs.CallTool(ctx, &mcp.CallToolParams{
				Name: "bases", Arguments: map[string]any{"action": "list"},
			})
			if err != nil {
				ошибки <- "вызов в сессии сорвался: " + err.Error()
				return
			}
			if len(res.Content) == 0 {
				ошибки <- "пустой ответ инструмента"
			}
		}(i)
	}

	wg.Wait()
	close(ошибки)
	for msg := range ошибки {
		t.Error(msg)
	}
}

func TestСетевойРежимТребуетТокен(t *testing.T) {
	endpoint := serveTest(t, "секрет")

	// Без токена подключение обязано быть отвергнуто.
	client := mcp.NewClient(&mcp.Implementation{Name: "no-token"}, nil)
	_, err := client.Connect(context.Background(),
		&mcp.StreamableClientTransport{Endpoint: endpoint}, nil)
	if err == nil {
		t.Fatal("подключение без токена прошло — токен не проверяется")
	}
	if !strings.Contains(err.Error(), "401") && !strings.Contains(err.Error(), "Unauthorized") {
		t.Errorf("отказ не похож на отказ авторизации: %v", err)
	}

	// С токеном — работает.
	cs := connectHTTP(t, endpoint, "секрет")
	if _, err := cs.ListTools(context.Background(), nil); err != nil {
		t.Errorf("с токеном подключение не работает: %v", err)
	}
}
