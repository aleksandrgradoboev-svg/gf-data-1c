// Пакет channel — канал к информационной базе: HTTP-обращение к расширению,
// установленному внутри базы.
//
// Главная работа пакета не в передаче байтов, а в РАЗЛИЧЕНИИ неудач. Три вида отказа
// требуют трёх разных действий человека, а выглядят одинаково — «база не ответила»:
//
//	соединение отвергнуто → веб-сервер не поднят (обычно после перезагрузки машины)
//	404 по маршруту       → расширение не установлено в этой базе
//	401/403               → пользователь базы не тот или прав не хватает
//
// Поэтому классификация живёт здесь, а не в каждом инструменте по отдельности.
package channel

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/Azure/go-ntlmssp"

	"github.com/greentech/gt-data-1c/internal/journal"
	"github.com/greentech/gt-data-1c/internal/refusal"
	"github.com/greentech/gt-data-1c/internal/registry"
)

// Client — канал к одной базе.
type Client struct {
	base registry.Base
	http *http.Client
}

// MaxResponseBytes — потолок ответа базы, 128 МиБ.
//
// Нужен не ради памяти, а ради внятности: ответ на порядок больше ожидаемого — это
// почти всегда ошибка запроса, и упереться в понятный отказ лучше, чем в загадочный
// сбой разбора где-то в середине гигабайта.
const MaxResponseBytes = 128 << 20

// New создаёт канал. Таймаут задан явно: агент, ждущий базу бесконечно, выглядит
// зависшим, а не отказавшим.
//
// Способ аутентификации выбирается по базе: доменной учётке (ДОМЕН\пользователь) нужен
// NTLM, и обычный Basic ей отвечает отказом прав, который читается как «не тот пароль».
func New(base registry.Base, timeout time.Duration) *Client {
	if timeout <= 0 {
		timeout = DefaultTimeout
	}

	client := &http.Client{Timeout: timeout}
	if useNTLM(base) {
		client.Transport = ntlmssp.Negotiator{RoundTripper: http.DefaultTransport}
	}
	return &Client{base: base, http: client}
}

// useNTLM решает, нужен ли доменный способ. Явное указание сильнее догадки, но и без
// него учётка с обратной косой чертой опознаётся как доменная: заставлять человека
// писать auth=ntlm там, где это видно по логину, — лишний повод для отказа прав.
func useNTLM(base registry.Base) bool {
	switch strings.ToLower(strings.TrimSpace(base.Auth)) {
	case "ntlm", "negotiate", "kerberos", "domain":
		return true
	case "basic":
		return false
	}
	return strings.Contains(base.User, `\`)
}

// DefaultTimeout — сколько ждать базу. Триста секунд не роскошь: выгрузка метаданных
// крупной конфигурации идёт минутами, и таймаут короче превращает медленный ответ
// в ложный «канал мёртв».
const DefaultTimeout = 300 * time.Second

// Base — база, к которой подключён канал.
func (c *Client) Base() registry.Base { return c.base }

// Get выполняет GET к методу расширения и возвращает тело ответа.
func (c *Client) Get(ctx context.Context, method string, query url.Values) ([]byte, error) {
	return c.do(ctx, http.MethodGet, method, query, nil)
}

// PostJSON выполняет POST с телом JSON — для запросов, которые не помещаются в URL.
func (c *Client) PostJSON(ctx context.Context, method string, payload any) ([]byte, error) {
	body, err := json.Marshal(payload)
	if err != nil {
		return nil, refusal.New(refusal.Internal, "запрос не сериализован", err.Error())
	}
	return c.do(ctx, http.MethodPost, method, nil, body)
}

// do — единственная точка выхода обращений к базе, и потому единственное место, где
// отказу проставляется её имя. Раскладывать Stamp по всем ветвям было бы то же самое,
// что просить не забывать: следующая добавленная ветвь про это не узнает.
func (c *Client) do(ctx context.Context, verb, method string, query url.Values, body []byte) ([]byte, error) {
	data, err := c.doRaw(ctx, verb, method, query, body)
	return data, refusal.Stamp(err, c.base.Name)
}

func (c *Client) doRaw(ctx context.Context, verb, method string, query url.Values, body []byte) ([]byte, error) {
	endpoint := strings.TrimRight(c.base.URL, "/") + "/" + strings.TrimLeft(method, "/")
	if len(query) > 0 {
		endpoint += "?" + query.Encode()
	}

	var reader io.Reader
	if body != nil {
		reader = strings.NewReader(string(body))
	}
	req, err := http.NewRequestWithContext(ctx, verb, endpoint, reader)
	if err != nil {
		return nil, refusal.New(refusal.BadRequest, "адрес базы негоден", err.Error(),
			"проверьте url базы: bases с action=list")
	}
	// Учётные данные подставляются здесь и только здесь: в URL реестра их нет,
	// поэтому в журнал и в сообщения об ошибках они не попадают.
	if c.base.User != "" {
		password, err := c.base.Secret()
		if err != nil {
			return nil, refusal.New(refusal.BadRequest, "пароль базы не прочитан", err.Error(),
				"перезапишите учётные данные: bases с action=add")
		}
		// Заголовок ставится одинаково для обоих способов: при NTLM его подхватывает
		// обёртка транспорта и превращает в доменный обмен.
		req.SetBasicAuth(c.base.User, password)
	}
	req.Header.Set("Accept", "application/json")
	if body != nil {
		req.Header.Set("Content-Type", "application/json; charset=utf-8")
	}

	started := time.Now()
	resp, err := c.http.Do(req)
	if err != nil {
		// В журнал идёт адрес без учётных данных: они живут отдельно от URL и в текст
		// запроса не подставляются.
		journal.Writef("%s %s %s → не отвечает: %v", c.base.Name, verb, endpoint, err)
		return nil, c.classifyTransport(err)
	}
	defer resp.Body.Close()
	journal.Writef("%s %s %s → %d за %s", c.base.Name, verb, endpoint,
		resp.StatusCode, time.Since(started).Round(time.Millisecond))

	// Читаем на байт больше потолка: превышение видно сразу, а не после того,
	// как ответ уже съел память.
	data, readErr := io.ReadAll(io.LimitReader(resp.Body, MaxResponseBytes+1))
	if readErr != nil {
		return nil, refusal.New(refusal.BaseError, "ответ базы не прочитан", readErr.Error())
	}
	if len(data) > MaxResponseBytes {
		return nil, refusal.New(refusal.BaseError, "ответ базы слишком велик",
			fmt.Sprintf("превышен потолок %d МиБ", MaxResponseBytes>>20),
			"сузьте запрос: перечислите нужные поля вместо звёздочки, поставьте limit, "+
				"добавьте отбор по периоду")
	}
	if err := c.classifyStatus(resp.StatusCode, data); err != nil {
		return nil, err
	}
	return data, nil
}

// classifyTransport различает «сервера нет» и прочие сетевые беды.
func (c *Client) classifyTransport(err error) error {
	var netErr net.Error
	if errors.As(err, &netErr) && netErr.Timeout() {
		return refusal.New(refusal.NoWebServer, "база не ответила вовремя", "истёк таймаут",
			"веб-сервер может быть занят перепроведением или выгрузкой",
			"проверьте канал инструментом probe")
	}
	var opErr *net.OpError
	if errors.As(err, &opErr) {
		return refusal.New(refusal.NoWebServer, "соединение с базой не установлено",
			"адрес не принимает подключение",
			"веб-сервер публикации 1С не поднят — он запускается процессом и после "+
				"перезагрузки машины исчезает молча",
			"проверьте канал инструментом probe")
	}
	return refusal.New(refusal.NoWebServer, "обращение к базе не удалось", err.Error(),
		"проверьте канал инструментом probe")
}

// classifyStatus превращает код ответа в отказ нужного вида.
func (c *Client) classifyStatus(code int, body []byte) error {
	switch {
	case code >= 200 && code < 300:
		return nil
	case code == http.StatusNotFound:
		return refusal.New(refusal.NoExtension, "расширение не отвечает в этой базе",
			"HTTP 404 по адресу сервиса",
			"расширение доступа к данным не установлено в базе "+c.base.Name,
			"либо публикация выполнена без HTTP-сервисов")
	case code == http.StatusUnauthorized || code == http.StatusForbidden:
		return refusal.New(refusal.Unauthorized, "база отказала в доступе",
			fmt.Sprintf("HTTP %d", code),
			"проверьте пользователя и пароль базы: bases с action=add перезапишет их",
			"пользователю нужна роль доступа расширения")
	default:
		return refusal.New(refusal.BaseError, "база ответила ошибкой",
			fmt.Sprintf("HTTP %d: %s", code, snippet(body)))
	}
}

func snippet(body []byte) string {
	s := strings.TrimSpace(string(body))
	if len(s) > 300 {
		return s[:300] + "…"
	}
	return s
}
