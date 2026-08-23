package channel

import (
	"context"
	"encoding/json"
	"net/url"
	"strings"

	"github.com/greentech/gt-data-1c/internal/refusal"
)

// Envelope — конверт ответа расширения. Поле ok есть всегда, поэтому отказ базы
// не приходится опознавать по составу полей.
type Envelope struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error"`
	Detail string `json:"detail"`
}

// Ask выполняет GET и разбирает ответ в out.
//
// Отказ базы (ok=false) превращается в refusal.BaseError: он приходит кодом 200
// намеренно — HTTP-коды заняты под состояния канала, — но для вызывающего это
// именно отказ, а не пустой результат.
func (c *Client) Ask(ctx context.Context, method string, query url.Values, out any) error {
	data, err := c.Get(ctx, method, query)
	if err != nil {
		return err
	}
	return unwrap(data, out)
}

// Tell выполняет POST с телом JSON и разбирает ответ в out.
func (c *Client) Tell(ctx context.Context, method string, payload, out any) error {
	data, err := c.PostJSON(ctx, method, payload)
	if err != nil {
		return err
	}
	return unwrap(data, out)
}

func unwrap(data []byte, out any) error {
	var env Envelope
	if err := json.Unmarshal(data, &env); err != nil {
		return refusal.New(refusal.BaseError, "ответ базы не разобран", err.Error(),
			"расширение вернуло не JSON — возможно, версия расширения старше сервера")
	}
	if !env.OK {
		what, detail := env.Error, env.Detail
		if strings.TrimSpace(what) == "" {
			// Ответ без признака ok и без причины: молчаливый отказ хуже громкого,
			// поэтому называем сам факт, а не печатаем пустую строку.
			what = "база ответила без признака успеха"
			detail = "в ответе нет ни ok:true, ни описания ошибки — вероятно, отвечает не наше расширение"
		}
		return refusal.New(refusal.BaseError, what, detail)
	}
	if out == nil {
		return nil
	}
	if err := json.Unmarshal(data, out); err != nil {
		return refusal.New(refusal.BaseError, "ответ базы не разобран", err.Error())
	}
	return nil
}
