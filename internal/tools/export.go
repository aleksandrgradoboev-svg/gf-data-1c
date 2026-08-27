package tools

import (
	"context"
	"encoding/csv"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/aleksandrgradoboev-svg/gt-data-1c/internal/refusal"
)

// pageSize — размер порции при выгрузке. Совпадает с потолком инструментов: больше
// одного запроса база всё равно не отдаёт.
const pageSize = 1000

// maxExportRows — предохранитель. Выгрузка в миллионы строк почти всегда означает
// забытый отбор, а не намерение: лучше упереться в понятный отказ, чем полчаса писать
// файл, который никто не откроет.
const maxExportRows = 500_000

type ExportInput struct {
	Base       string         `json:"base" jsonschema:"Имя базы 1С из реестра. Обязательно; перечень — bases с action=list"`
	Query      string         `json:"query" jsonschema:"Текст запроса на языке 1С. Только ВЫБРАТЬ. Для устойчивой выгрузки добавьте УПОРЯДОЧИТЬ"`
	Parameters map[string]any `json:"parameters,omitempty" jsonschema:"Параметры запроса: ключ без амперсанда"`
	Format     string         `json:"format,omitempty" jsonschema:"Формат файла: csv (умолчание) или jsonl"`
	Path       string         `json:"path,omitempty" jsonschema:"Куда записать файл. Пусто — каталог выгрузок в профиле пользователя"`
	MaxRows    int            `json:"max_rows,omitempty" jsonschema:"Предохранитель: остановиться после стольких строк (по умолчанию 500000)"`
}

func ExportTool() *mcp.Tool {
	return &mcp.Tool{
		Name: "export",
		Description: "Выгрузить результат запроса целиком в файл (csv или jsonl), не пропуская его " +
			"через ответ инструмента. Нужен, когда строк больше тысячи: обычный query отдаёт " +
			"страницу, а здесь сервер сам обходит все страницы и пишет файл, возвращая путь и " +
			"число строк. Ссылки в csv печатаются представлением, в jsonl — объектом с типом и " +
			"идентификатором. Для устойчивой выгрузки запрос должен содержать УПОРЯДОЧИТЬ.",
	}
}

func (s *Set) Export(ctx context.Context, _ *mcp.CallToolRequest, in ExportInput) (*mcp.CallToolResult, any, error) {
	if strings.TrimSpace(in.Query) == "" {
		return nil, nil, refusal.New(refusal.BadRequest, "текст запроса пуст", "поле query обязательно")
	}
	client, err := s.channelFor(in.Base)
	if err != nil {
		return nil, nil, err
	}

	format := strings.ToLower(strings.TrimSpace(in.Format))
	if format == "" {
		format = "csv"
	}
	if format != "csv" && format != "jsonl" {
		return nil, nil, refusal.New(refusal.BadRequest, "формат не распознан",
			"format="+in.Format, "допустимо: csv, jsonl")
	}

	limit := in.MaxRows
	if limit <= 0 {
		limit = maxExportRows
	}

	path, err := exportPath(in.Path, client.Base().Name, format)
	if err != nil {
		return nil, nil, err
	}
	file, err := os.Create(path)
	if err != nil {
		return nil, nil, refusal.New(refusal.Internal, "файл выгрузки не создан", err.Error())
	}
	defer file.Close()

	var (
		writer     *csv.Writer
		encoder    *json.Encoder
		columns    []string
		written    int
		offset     int
		totalKnown int
	)
	if format == "csv" {
		writer = csv.NewWriter(file)
		writer.Comma = ';' // Excel в русской локали ждёт точку с запятой
		defer writer.Flush()
	} else {
		encoder = json.NewEncoder(file)
	}

	started := time.Now()
	for {
		payload := map[string]any{"query": in.Query, "limit": pageSize, "offset": offset}
		if len(in.Parameters) > 0 {
			payload["parameters"] = in.Parameters
		}

		var reply queryReply
		if err := client.Tell(ctx, "query", payload, &reply); err != nil {
			os.Remove(path)
			return nil, nil, err
		}
		totalKnown = reply.ВсегоСтрок

		if written == 0 {
			columns = reply.Колонки
			if writer != nil {
				if err := writer.Write(columns); err != nil {
					return nil, nil, refusal.New(refusal.Internal, "заголовок не записан", err.Error())
				}
			}
		}

		for _, row := range reply.Строки {
			if err := writeRow(writer, encoder, columns, row); err != nil {
				return nil, nil, err
			}
			written++
			if written >= limit {
				break
			}
		}

		if !reply.ЕстьЕщё || written >= limit || len(reply.Строки) == 0 {
			offset = reply.СледующееСмещение
			break
		}
		offset = reply.СледующееСмещение
	}

	if writer != nil {
		writer.Flush()
		if err := writer.Error(); err != nil {
			return nil, nil, refusal.New(refusal.Internal, "файл выгрузки не дописан", err.Error())
		}
	}

	var b strings.Builder
	fmt.Fprintf(&b, "Выгружено строк: %d", written)
	if totalKnown > 0 {
		fmt.Fprintf(&b, " из %d", totalKnown)
	}
	fmt.Fprintf(&b, "\nФайл: %s\nФормат: %s, колонок %d, время %s",
		path, format, len(columns), time.Since(started).Round(time.Millisecond))
	if written >= limit && totalKnown > written {
		fmt.Fprintf(&b, "\n⚠ Выгрузка остановлена предохранителем на %d строках, в результате их %d. "+
			"Это не весь результат: уточните отбор или поднимите max_rows.", limit, totalKnown)
	}
	return text(b.String()), nil, nil
}

// writeRow пишет строку в выбранном формате.
//
// CSV разворачивает ссылку в представление: файл открывают глазами в таблице, и объект
// с типом и идентификатором там только мешает. JSONL сохраняет всё — его читает программа.
func writeRow(writer *csv.Writer, encoder *json.Encoder, columns []string, row map[string]any) error {
	if writer != nil {
		values := make([]string, 0, len(columns))
		for _, col := range columns {
			values = append(values, csvValue(row[col]))
		}
		if err := writer.Write(values); err != nil {
			return refusal.New(refusal.Internal, "строка не записана", err.Error())
		}
		return nil
	}
	if err := encoder.Encode(row); err != nil {
		return refusal.New(refusal.Internal, "строка не записана", err.Error())
	}
	return nil
}

func csvValue(value any) string {
	if ref, ok := value.(map[string]any); ok {
		представление, _ := ref["представление"].(string)
		return представление
	}
	if value == nil {
		return ""
	}
	return renderValue(value)
}

// exportPath выбирает имя файла: заданное вызывающим либо своё, с базой и временем.
func exportPath(explicit, base, format string) (string, error) {
	if strings.TrimSpace(explicit) != "" {
		if err := os.MkdirAll(filepath.Dir(explicit), 0o700); err != nil {
			return "", refusal.New(refusal.Internal, "каталог выгрузки не создан", err.Error())
		}
		return explicit, nil
	}

	dir, err := os.UserCacheDir()
	if err != nil || dir == "" {
		dir = "."
	}
	dir = filepath.Join(dir, "gt-data-1c", "exports")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", refusal.New(refusal.Internal, "каталог выгрузок не создан", err.Error())
	}
	name := fmt.Sprintf("%s-%s.%s", base, time.Now().Format("20060102-150405"), format)
	return filepath.Join(dir, name), nil
}
