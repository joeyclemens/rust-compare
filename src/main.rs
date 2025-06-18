use actix_files as fs;
use actix_web::{web, App, HttpResponse, HttpServer, Responder, Error};
use actix_multipart::Multipart;
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::collections::HashMap;
use std::io::Cursor;
use calamine::{Reader, Xlsx};
use rust_xlsxwriter::{Workbook, Format, FormatBorder, FormatAlign};
use itertools::Itertools;
use webbrowser;
use rfd::FileDialog;
use open;

// Structure to store uploaded file information
#[derive(Default)]
struct AppState {
    files: Mutex<HashMap<String, Vec<String>>>, // filename -> sheet names
    selected_sheets: Mutex<Option<((String, String), (String, String))>>, // ((file1_filename, file1_sheet), (file2_filename, file2_sheet))
    file_data: Mutex<HashMap<String, HashMap<String, Vec<Vec<String>>>>>, // filename -> (sheet_name -> rows)
    column_matches: Mutex<Vec<ColumnMatch>>, // New field to store column matches
}

#[derive(Serialize, Deserialize, Clone)] // Add Clone trait
struct ColumnMatch {
    file1_column: String,
    file2_column: String,
}

#[derive(Serialize, Deserialize)]
struct UploadResponse {
    filename: String,
    sheets: Vec<String>,
}

#[derive(Serialize)]
struct UploadedFilesResponse {
    file1: Option<FileInfo>,
    file2: Option<FileInfo>,
}

#[derive(Serialize)]
struct FileInfo {
    filename: String,
    sheets: Vec<String>,
}

#[derive(Serialize)]
struct ColumnHeadersResponse {
    file1_headers: Vec<String>,
    file2_headers: Vec<String>,
}

#[derive(Deserialize)]
struct SheetSelection {
    file1_filename: String,
    file1_sheet: String,
    file2_filename: String,
    file2_sheet: String,
}

#[derive(Serialize)]
struct SummaryData {
    file1_filename: String,
    file1_sheet: String,
    file2_filename: String,
    file2_sheet: String,
    column_matches: Vec<ColumnMatch>,
}

#[derive(Serialize)]
struct ComparisonResult {
    match_count: usize,
    left_only_count: usize,
    right_only_count: usize,
    total_rows: usize,
    output_path: String,
    file1_filename: String,
    file1_sheet: String,
    file2_filename: String,
    file2_sheet: String,
}

#[derive(Serialize)]
struct ComparisonError {
    error: String,
}

#[derive(Deserialize)]
struct ComparisonRequest {
    save_location: Option<String>,
    output_filename: Option<String>,
}

#[derive(Serialize)]
struct DirectoryResponse {
    path: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct OpenFileResponse {
    success: bool,
    error: Option<String>,
}

async fn index() -> impl Responder {
    HttpResponse::Ok().body(include_str!("../templates/index.html"))
}

async fn sheet_selection() -> impl Responder {
    HttpResponse::Ok().body(include_str!("../templates/sheet_selection.html"))
}

async fn column_matching() -> impl Responder {
    HttpResponse::Ok().body(include_str!("../templates/column_matching.html"))
}

// New route for the summary page
async fn summary_page() -> impl Responder {
    HttpResponse::Ok().body(include_str!("../templates/summary_page.html"))
}

// Add new route for results page
async fn results_page() -> impl Responder {
    HttpResponse::Ok().body(include_str!("../templates/results_page.html"))
}

async fn get_uploaded_files(state: web::Data<AppState>) -> impl Responder {
    let files = state.files.lock().unwrap();
    let mut response = UploadedFilesResponse {
        file1: None,
        file2: None,
    };
    let mut filenames: Vec<&String> = files.keys().collect();
    filenames.sort();

    if let Some(filename) = filenames.get(0) {
        if let Some(sheets) = files.get(*filename) {
            response.file1 = Some(FileInfo {
                filename: (**filename).clone(),
                sheets: sheets.clone(),
            });
        }
    }
    if let Some(filename) = filenames.get(1) {
        if let Some(sheets) = files.get(*filename) {
            response.file2 = Some(FileInfo {
                filename: (**filename).clone(),
                sheets: sheets.clone(),
            });
        }
    }

    HttpResponse::Ok().json(response)
}

async fn get_column_headers(state: web::Data<AppState>) -> impl Responder {
    let selected_sheets = state.selected_sheets.lock().unwrap();
    let file_data = state.file_data.lock().unwrap();
    if let Some(((file1_filename, file1_sheet), (file2_filename, file2_sheet))) = selected_sheets.as_ref() {
        let mut response = ColumnHeadersResponse {
            file1_headers: Vec::new(),
            file2_headers: Vec::new(),
        };
        if let Some(sheet_map) = file_data.get(file1_filename) {
            if let Some(rows) = sheet_map.get(file1_sheet) {
                if !rows.is_empty() {
                    response.file1_headers = rows[0].clone();
                }
            }
        }
        if let Some(sheet_map) = file_data.get(file2_filename) {
            if let Some(rows) = sheet_map.get(file2_sheet) {
                if !rows.is_empty() {
                    response.file2_headers = rows[0].clone();
                }
            }
        }
        HttpResponse::Ok().json(response)
    } else {
        HttpResponse::BadRequest().body("No sheets selected")
    }
}

async fn select_sheets(
    state: web::Data<AppState>,
    selection: web::Json<SheetSelection>,
) -> impl Responder {
    *state.selected_sheets.lock().unwrap() = Some(((
        selection.file1_filename.clone(),
        selection.file1_sheet.clone(),
    ), (
        selection.file2_filename.clone(),
        selection.file2_sheet.clone(),
    )));
    HttpResponse::Ok().json(serde_json::json!({ "success": true })) // Return JSON success
}

// New endpoint to save column matches
async fn save_column_matches(
    state: web::Data<AppState>,
    matches: web::Json<Vec<ColumnMatch>>,
) -> impl Responder {
    *state.column_matches.lock().unwrap() = matches.into_inner();
    HttpResponse::Ok().json(serde_json::json!({ "success": true }))
}

// New endpoint to get summary data
async fn get_summary_data(state: web::Data<AppState>) -> impl Responder {
    let selected_sheets = state.selected_sheets.lock().unwrap();
    let column_matches = state.column_matches.lock().unwrap();

    if let Some(((file1_filename, file1_sheet), (file2_filename, file2_sheet))) = selected_sheets.as_ref() {
        let summary = SummaryData {
            file1_filename: file1_filename.clone(),
            file1_sheet: file1_sheet.clone(),
            file2_filename: file2_filename.clone(),
            file2_sheet: file2_sheet.clone(),
            column_matches: column_matches.clone(),
        };
        HttpResponse::Ok().json(summary)
    } else {
        HttpResponse::BadRequest().body("Summary data not available. Please complete previous steps.")
    }
}

async fn upload_file(
    state: web::Data<AppState>,
    mut payload: Multipart,
) -> Result<HttpResponse, Error> {
    log::info!("Upload handler called");
    let mut found_field = false;
    while let Ok(Some(mut field)) = payload.try_next().await {
        found_field = true;
        let filename = field.content_disposition().get_filename().unwrap_or("unknown").to_string();
        log::info!("Received field with filename: {}", filename);
        let mut data = Vec::new();
        
        while let Some(chunk) = field.try_next().await? {
            log::info!("Received chunk of size: {}", chunk.len());
            data.extend_from_slice(&chunk);
        }
        log::info!("Total file size received: {} bytes", data.len());

        // Read Excel file
        let cursor = Cursor::new(data);
        match Xlsx::new(cursor) {
            Ok(mut workbook) => {
                let sheet_names: Vec<String> = workbook
                    .sheet_names()
                    .to_owned()
                    .into_iter()
                    .collect();
                log::info!("Parsed Excel file '{}', sheets: {:?}", filename, sheet_names);
                // Store sheet names in state
                state.files.lock().unwrap().insert(filename.clone(), sheet_names.clone());

                // Read and store sheet data
                let mut sheet_data = HashMap::new();
                for sheet_name in &sheet_names {
                    if let Some(Ok(range)) = workbook.worksheet_range(sheet_name) {
                        let mut rows = Vec::new();
                        for row in range.rows() {
                            let row_data: Vec<String> = row
                                .iter()
                                .map(|cell| cell.to_string().trim().to_string())
                                .collect();
                            rows.push(row_data);
                        }
                        sheet_data.insert(sheet_name.clone(), rows);
                    }
                }
                state.file_data.lock().unwrap().insert(filename.clone(), sheet_data);

                return Ok(HttpResponse::Ok().json(UploadResponse {
                    filename,
                    sheets: sheet_names,
                }));
            }
            Err(e) => {
                log::error!("Failed to parse Excel file '{}': {}", filename, e);
                return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                    "error": format!("Failed to parse Excel file: {}", e)
                })));
            }
        }
    }
    if !found_field {
        log::error!("No fields found in multipart payload");
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": "No file was uploaded"
        })));
    }
    log::error!("No valid Excel file found in upload");
    Ok(HttpResponse::BadRequest().json(serde_json::json!({
        "error": "Failed to process file"
    })))
}

// New endpoint to perform the comparison
async fn perform_comparison(
    state: web::Data<AppState>,
    req: web::Json<ComparisonRequest>,
) -> impl Responder {
    log::info!("Starting comparison process");
    let selected_sheets = state.selected_sheets.lock().unwrap();
    let column_matches = state.column_matches.lock().unwrap();
    let file_data = state.file_data.lock().unwrap();

    if let Some(((file1_filename, file1_sheet), (file2_filename, file2_sheet))) = selected_sheets.as_ref() {
        log::info!("Selected sheets: File1={} ({}), File2={} ({})", 
            file1_filename, file1_sheet, file2_filename, file2_sheet);

        // Get the data for both files
        let file1_data = match file_data.get(file1_filename) {
            Some(sheet_map) => match sheet_map.get(file1_sheet) {
                Some(rows) => {
                    log::info!("Found {} rows in file1", rows.len());
                    rows
                },
                None => {
                    log::error!("Sheet {} not found in file {}", file1_sheet, file1_filename);
                    return HttpResponse::BadRequest().json(ComparisonError { 
                        error: format!("Sheet {} not found in file {}", file1_sheet, file1_filename) 
                    });
                },
            },
            None => {
                log::error!("File {} not found", file1_filename);
                return HttpResponse::BadRequest().json(ComparisonError { 
                    error: format!("File {} not found", file1_filename) 
                });
            },
        };

        let file2_data = match file_data.get(file2_filename) {
            Some(sheet_map) => match sheet_map.get(file2_sheet) {
                Some(rows) => {
                    log::info!("Found {} rows in file2", rows.len());
                    rows
                },
                None => {
                    log::error!("Sheet {} not found in file {}", file2_sheet, file2_filename);
                    return HttpResponse::BadRequest().json(ComparisonError { 
                        error: format!("Sheet {} not found in file {}", file2_sheet, file2_filename) 
                    });
                },
            },
            None => {
                log::error!("File {} not found", file2_filename);
                return HttpResponse::BadRequest().json(ComparisonError { 
                    error: format!("File {} not found", file2_filename) 
                });
            },
        };

        // Get headers from both files
        let file1_headers = &file1_data[0];
        let file2_headers = &file2_data[0];

        // Create column index mappings
        let mut file1_col_indices = HashMap::new();
        let mut file2_col_indices = HashMap::new();
        
        for (i, header) in file1_headers.iter().enumerate() {
            file1_col_indices.insert(header.clone(), i);
        }
        
        for (i, header) in file2_headers.iter().enumerate() {
            file2_col_indices.insert(header.clone(), i);
        }

        // Create a new workbook for the output
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        
        // Create formats
        let header_format = Format::new()
            .set_bold()
            .set_background_color("#182643")
            .set_font_color("#FFFFFF")
            .set_font_name("General Sans")
            .set_font_size(12)
            .set_align(FormatAlign::Center)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap();

        let body_format = Format::new()
            .set_font_name("General Sans")
            .set_font_size(9)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap();

        let number_format = Format::new()
            .set_font_name("General Sans")
            .set_font_size(9)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap()
            .set_num_format("#,##0");

        let currency_format = Format::new()
            .set_font_name("General Sans")
            .set_font_size(9)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap()
            .set_num_format("£#,##0.00");

        // Add headers with formatting
        let mut col = 0;
        
        // Set header row height
        if let Err(e) = sheet.set_row_height(0, 43.5) {
            return HttpResponse::InternalServerError().json(ComparisonError { 
                error: format!("Failed to set row height: {}", e) 
            });
        }
        
        // Helper function to check if a header contains quantity or cost keywords
        let is_quantity_column = |header: &str| {
            let header_lower = header.to_lowercase();
            header_lower.contains("qty") || header_lower.contains("quantity")
        };

        let is_cost_column = |header: &str| {
            let header_lower = header.to_lowercase();
            header_lower.contains("cost")
        };
        
        // Add file1 columns
        for header in file1_headers {
            if let Err(e) = sheet.write_string_with_format(0, col, &format!("File1_{}", header), &header_format) {
                return HttpResponse::InternalServerError().json(ComparisonError { 
                    error: format!("Failed to write header: {}", e) 
                });
            }
            // Set column width based on header length
            if let Err(e) = sheet.set_column_width(col, (header.len() + 10) as f64) {
                return HttpResponse::InternalServerError().json(ComparisonError { 
                    error: format!("Failed to set column width: {}", e) 
                });
            }
            col += 1;
        }
        
        // Add file2 columns
        for header in file2_headers {
            if let Err(e) = sheet.write_string_with_format(0, col, &format!("File2_{}", header), &header_format) {
                return HttpResponse::InternalServerError().json(ComparisonError { 
                    error: format!("Failed to write header: {}", e) 
                });
            }
            // Set column width based on header length
            if let Err(e) = sheet.set_column_width(col, (header.len() + 10) as f64) {
                return HttpResponse::InternalServerError().json(ComparisonError { 
                    error: format!("Failed to set column width: {}", e) 
                });
            }
            col += 1;
        }
        
        // Add match status column
        if let Err(e) = sheet.write_string_with_format(0, col, "Match_Status", &header_format) {
            return HttpResponse::InternalServerError().json(ComparisonError { 
                error: format!("Failed to write header: {}", e) 
            });
        }
        if let Err(e) = sheet.set_column_width(col, 15.0) {
            return HttpResponse::InternalServerError().json(ComparisonError { 
                error: format!("Failed to set column width: {}", e) 
            });
        }

        // Create a map to store unique rows from both files
        let mut unique_rows = HashMap::new();
        let mut row_index = 1;

        // Process file1 data
        for row in file1_data.iter().skip(1) {
            let mut row_key = Vec::new();
            for col_match in column_matches.iter() {
                if let Some(&idx) = file1_col_indices.get(&col_match.file1_column) {
                    row_key.push(row[idx].clone());
                }
            }
            let row_key = row_key.join("|");
            unique_rows.insert(row_key, (row.clone(), "left_only".to_string()));
        }

        // Process file2 data and update matches
        for row in file2_data.iter().skip(1) {
            let mut row_key = Vec::new();
            for col_match in column_matches.iter() {
                if let Some(&idx) = file2_col_indices.get(&col_match.file2_column) {
                    row_key.push(row[idx].clone());
                }
            }
            let row_key = row_key.join("|");
            
            match unique_rows.get_mut(&row_key) {
                Some((_, status)) => *status = "both".to_string(),
                None => {
                    unique_rows.insert(row_key, (row.clone(), "right_only".to_string()));
                }
            }
        }

        // Write data to the output file
        let mut match_count = 0;
        let mut left_only_count = 0;
        let mut right_only_count = 0;

        for (_, (row, status)) in unique_rows.iter() {
            let mut col = 0;
            
            // Write file1 data
            for header in file1_headers {
                if let Some(&idx) = file1_col_indices.get(header) {
                    let value = if status == "right_only" {
                        // Leave empty for rows unique to file2
                        ""
                    } else {
                        &row[idx]
                    };
                    
                    if is_quantity_column(header) {
                        if !value.is_empty() {
                            if let Ok(num) = value.parse::<f64>() {
                                if let Err(e) = sheet.write_number_with_format(row_index, col, num, &number_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            } else {
                                if let Err(e) = sheet.write_string_with_format(row_index, col, value, &body_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            }
                        }
                    } else if is_cost_column(header) {
                        if !value.is_empty() {
                            if let Ok(num) = value.parse::<f64>() {
                                if let Err(e) = sheet.write_number_with_format(row_index, col, num, &currency_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            } else {
                                if let Err(e) = sheet.write_string_with_format(row_index, col, value, &body_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            }
                        }
                    } else if !value.is_empty() {
                        if let Err(e) = sheet.write_string_with_format(row_index, col, value, &body_format) {
                            return HttpResponse::InternalServerError().json(ComparisonError { 
                                error: format!("Failed to write data: {}", e) 
                            });
                        }
                    }
                }
                col += 1;
            }
            
            // Write file2 data
            for header in file2_headers {
                if let Some(&idx) = file2_col_indices.get(header) {
                    let value = if status == "left_only" {
                        // Leave empty for rows unique to file1
                        ""
                    } else {
                        &row[idx]
                    };
                    
                    if is_quantity_column(header) {
                        if !value.is_empty() {
                            if let Ok(num) = value.parse::<f64>() {
                                if let Err(e) = sheet.write_number_with_format(row_index, col, num, &number_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            } else {
                                if let Err(e) = sheet.write_string_with_format(row_index, col, value, &body_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            }
                        }
                    } else if is_cost_column(header) {
                        if !value.is_empty() {
                            if let Ok(num) = value.parse::<f64>() {
                                if let Err(e) = sheet.write_number_with_format(row_index, col, num, &currency_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            } else {
                                if let Err(e) = sheet.write_string_with_format(row_index, col, value, &body_format) {
                                    return HttpResponse::InternalServerError().json(ComparisonError { 
                                        error: format!("Failed to write data: {}", e) 
                                    });
                                }
                            }
                        }
                    } else if !value.is_empty() {
                        if let Err(e) = sheet.write_string_with_format(row_index, col, value, &body_format) {
                            return HttpResponse::InternalServerError().json(ComparisonError { 
                                error: format!("Failed to write data: {}", e) 
                            });
                        }
                    }
                }
                col += 1;
            }
            
            // Write match status
            if let Err(e) = sheet.write_string_with_format(row_index, col, status, &body_format) {
                return HttpResponse::InternalServerError().json(ComparisonError { 
                    error: format!("Failed to write status: {}", e) 
                });
            }
            
            // Update counts
            match status.as_str() {
                "both" => match_count += 1,
                "left_only" => left_only_count += 1,
                "right_only" => right_only_count += 1,
                _ => {}
            }
            
            row_index += 1;
        }

        // Add a summary sheet with the same formatting
        let summary_sheet = workbook.add_worksheet();
        
        // Set header row height for summary sheet
        if let Err(e) = summary_sheet.set_row_height(0, 43.5) {
            return HttpResponse::InternalServerError().json(ComparisonError { 
                error: format!("Failed to set row height: {}", e) 
            });
        }

        if let Err(e) = summary_sheet.write_string_with_format(0, 0, "Comparison Summary", &header_format) {
            return HttpResponse::InternalServerError().json(ComparisonError { 
                error: format!("Failed to write summary: {}", e) 
            });
        }

        // Write summary data
        let summary_data = [
            ("Matching Rows", match_count),
            ("Rows Unique to File 1", left_only_count),
            ("Rows Unique to File 2", right_only_count),
            ("Total Rows", unique_rows.len()),
        ];

        for (i, (label, value)) in summary_data.iter().enumerate() {
            if let Err(e) = summary_sheet.write_string_with_format((i + 1) as u32, 0, *label, &body_format) {
                return HttpResponse::InternalServerError().json(ComparisonError { 
                    error: format!("Failed to write summary: {}", e) 
                });
            }
            if let Err(e) = summary_sheet.write_number_with_format((i + 1) as u32, 1, *value as f64, &body_format) {
                return HttpResponse::InternalServerError().json(ComparisonError { 
                    error: format!("Failed to write summary: {}", e) 
                });
            }
        }

        // Determine output path
        log::info!("Save location requested: {:?}", req.save_location);
        let output_path = match req.save_location.as_ref() {
            Some(path) if !path.trim().is_empty() => {
                let path = std::path::Path::new(path.trim());
                log::info!("Attempting to save to: {}", path.display());
                
                // Create parent directory if it doesn't exist
                if let Some(parent) = path.parent() {
                    if !parent.exists() {
                        log::info!("Creating directory: {}", parent.display());
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            log::error!("Failed to create directory: {}", e);
                            return HttpResponse::InternalServerError().json(ComparisonError { 
                                error: format!("Failed to create directory '{}': {}", parent.display(), e) 
                            });
                        }
                    }
                }

                // Create the file
                if let Err(e) = std::fs::File::create(path) {
                    log::error!("Failed to create output file: {}", e);
                    return HttpResponse::InternalServerError().json(ComparisonError { 
                        error: format!("Failed to create output file '{}': {}", path.display(), e) 
                    });
                }
                path.to_string_lossy().into_owned()
            },
            _ => {
                let default_path = std::path::Path::new("comparison_output.xlsx");
                log::info!("Using default save location: {}", default_path.display());
                if let Err(e) = std::fs::File::create(default_path) {
                    log::error!("Failed to create default output file: {}", e);
                    return HttpResponse::InternalServerError().json(ComparisonError { 
                        error: format!("Failed to create output file '{}': {}", default_path.display(), e) 
                    });
                }
                default_path.to_string_lossy().into_owned()
            },
        };

        // Save the workbook
        log::info!("Saving workbook to: {}", output_path);
        if let Err(e) = workbook.save(&output_path) {
            log::error!("Failed to save workbook: {}", e);
            return HttpResponse::InternalServerError().json(ComparisonError { 
                error: format!("Failed to save workbook to '{}': {}", output_path, e) 
            });
        }
        log::info!("Workbook saved successfully");

        let result = ComparisonResult {
            match_count,
            left_only_count,
            right_only_count,
            total_rows: unique_rows.len(),
            output_path,
            file1_filename: file1_filename.clone(),
            file1_sheet: file1_sheet.clone(),
            file2_filename: file2_filename.clone(),
            file2_sheet: file2_sheet.clone(),
        };

        log::info!("Comparison completed successfully");
        HttpResponse::Ok().json(result)
    } else {
        log::error!("No sheets selected for comparison");
        HttpResponse::BadRequest().json(ComparisonError { 
            error: "No sheets selected for comparison".to_string() 
        })
    }
}

// Update endpoint to use file save dialog
async fn select_directory() -> impl Responder {
    match FileDialog::new()
        .set_title("Save Comparison File")
        .add_filter("Excel Files", &["xlsx"])
        .set_directory(std::env::current_dir().unwrap_or_default())
        .save_file() {
            Some(path) => HttpResponse::Ok().json(DirectoryResponse {
                path: Some(path.to_string_lossy().into_owned()),
                error: None,
            }),
            None => HttpResponse::Ok().json(DirectoryResponse {
                path: None,
                error: Some("No file selected".to_string()),
            }),
    }
}

// Add new endpoint to open file
async fn open_output_file(path: web::Query<HashMap<String, String>>) -> impl Responder {
    if let Some(file_path) = path.get("path") {
        match open::that(file_path) {
            Ok(_) => HttpResponse::Ok().json(OpenFileResponse {
                success: true,
                error: None,
            }),
            Err(e) => HttpResponse::InternalServerError().json(OpenFileResponse {
                success: false,
                error: Some(format!("Failed to open file: {}", e)),
            }),
        }
    } else {
        HttpResponse::BadRequest().json(OpenFileResponse {
            success: false,
            error: Some("No file path provided".to_string()),
        })
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let app_state = web::Data::new(AppState::default());
    let server_url = "http://127.0.0.1:8080";

    // Start the server
    let server = HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .service(fs::Files::new("/static", "./static").show_files_listing())
            .route("/", web::get().to(index))
            .route("/sheet-selection", web::get().to(sheet_selection))
            .route("/column-matching", web::get().to(column_matching))
            .route("/summary", web::get().to(summary_page))
            .route("/results", web::get().to(results_page))
            .route("/upload", web::post().to(upload_file))
            .route("/uploaded-files", web::get().to(get_uploaded_files))
            .route("/column-headers", web::get().to(get_column_headers))
            .route("/select-sheets", web::post().to(select_sheets))
            .route("/save-column-matches", web::post().to(save_column_matches))
            .route("/summary-data", web::get().to(get_summary_data))
            .route("/compare", web::post().to(perform_comparison))
            .route("/select-directory", web::get().to(select_directory))
            .route("/open-file", web::get().to(open_output_file))
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    // Open the browser
    if let Err(e) = webbrowser::open(server_url) {
        log::error!("Failed to open browser: {}", e);
    }

    server.await
}
