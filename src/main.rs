/*	Copyright (c) 2023 Laurenz Werner
	
	This file is part of OpenJeopardy.
	
	OpenJeopardy is free software: you can redistribute it and/or modify
	it under the terms of the GNU General Public License as published by
	the Free Software Foundation, either version 3 of the License, or
	(at your option) any later version.
	
	OpenJeopardy is distributed in the hope that it will be useful,
	but WITHOUT ANY WARRANTY; without even the implied warranty of
	MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
	GNU General Public License for more details.
	
	You should have received a copy of the GNU General Public License
	along with OpenJeopardy.  If not, see <http://www.gnu.org/licenses/>.
*/

#[macro_use]
extern crate lazy_static;

use std::sync::{Arc, RwLock};
use actix_web::{get, post, delete, web, App, HttpRequest, HttpResponse, HttpServer, Responder, web::Redirect};
use moka::future::Cache;
use serde::Deserialize;
use regex::Regex;

const SERVER_ADDRESS: &str = "0.0.0.0";
const SERVER_PORT: u16 = 4242;

const SITE_HEADER: &str = "
<html>
	<head>
		<title>Jeopardy</title>
		<style>
			.buzzer {
				background-color: #ee2210;
				border: none;
				border-radius: 42px;
				color: #fff;
				width: 90vw;
				height: 95vh;
				text-decoration: none;
				margin-top: 10px;
				margin-bottom: 10px;
				margin-left: 5vw;
				margin-right: 5vw;
				cursor: pointer;
				font-size: 8vw;
			}
			.regular {
				background-color: #3a5eff;
				border: none;
				color: #fff;
				width: 90vw;
				height: 100px;
				text-decoration: none;
				margin-top: 10px;
				margin-bottom: 10px;
				margin-left: 3vw;
				margin-right: 3vw;
				cursor: pointer;
			}
			.pad {
				margin-left: 3vw;
				margin-right: 3vw;
			}
			input[type=text] {
				margin-left: 3vw;
				margin-right: 3vw;
				width: 90vw;
			}
		</style>
	</head>
	<body>";
const SITE_FOOTER: &str = "	</body>
</html>";

lazy_static! {
	static ref IS_VALID_NAME: Regex = Regex::new("^[0-9a-zA-Z_-]+$").unwrap();

}

#[derive(Deserialize)]
struct RegisterQuery {
	name: String,
}

#[derive(Deserialize)]
struct AdminQuery {
	setstate: u16,
	reset: bool,
}

enum Status {
	Registration,
	BuzzerActive,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let status = Arc::new(RwLock::new(Status::Registration));
	let ip_cache = Cache::<String, String>::builder().build();
	HttpServer::new(move || {
		App::new()
			.app_data(web::Data::new(status.clone()))
			.app_data(web::Data::new(ip_cache.clone()))
			.service(register)
			.service(buzz)
			.service(admin)
			.service(splash)
			.service(buzzer)
	})
	.bind((SERVER_ADDRESS, SERVER_PORT))?
	.run()
	.await
}

#[get("/register")]
async fn register(req: HttpRequest, query: web::Query<RegisterQuery>, status: web::Data<Arc<RwLock<Status>>>, ip_cache: web::Data<Cache<String, String>>) -> impl Responder {
	match *(status.read().unwrap()) {
		Status::Registration => {},
		_ => {
			return HttpResponse::BadRequest().body("Game has already started! Try again later.");
		}
	};
	if !IS_VALID_NAME.is_match(&query.name) {
		return HttpResponse::BadRequest().body("Invalid name".as_bytes());
	}
	let ip: String = match req.peer_addr() {
		Some(res) => format!("{}", res.ip()),
		None => return HttpResponse::InternalServerError().body("Could not get IP address".as_bytes())
	};
	ip_cache.insert(ip.clone(), query.name.clone()).await;
	println!("{} registered using name \"{}\"", ip, query.name);
	HttpResponse::TemporaryRedirect().insert_header(("location", "/buzzer")).finish()
}

#[get("/buzz")]
async fn buzz(req: HttpRequest, ip_cache: web::Data<Cache<String, String>>) -> impl Responder {
	let ip: String = match req.peer_addr() {
		Some(res) => format!("{}", res.ip()),
		None => return HttpResponse::InternalServerError().body("Could not get IP address".as_bytes())
	};
	let name = ip_cache.get(&ip).await;
	if name.is_none() {
		return HttpResponse::BadRequest().body("Not registered".as_bytes());
	}
	println!("{} buzzered!", name.unwrap());
	HttpResponse::TemporaryRedirect().insert_header(("location", "/buzzer")).finish()
}

#[get("/admin")]
async fn admin(req: HttpRequest) -> impl Responder {
	let ip = match req.peer_addr() {
		Some(res) => res.ip(),
		None => return HttpResponse::InternalServerError().body("Could not get IP address".as_bytes())
	};
	if !ip.is_loopback() {
		return HttpResponse::Unauthorized().body("Not an admin".as_bytes());
	}
	HttpResponse::Ok().finish()
}

#[get("/")]
async fn splash() -> impl Responder {
	let site = format!("{}<h1 class=\"pad\">Willkommen zum Jeopardy! Gib dir einen Namen und registriere dich!</h1>
	<form action=\"/register\">
		<input type=\"text\" id=\"name\" name=\"name\">
		<input type=\"submit\" class=\"regular\" value=\"Registrieren\">
	</form>{}", SITE_HEADER, SITE_FOOTER).into_bytes();
	HttpResponse::Ok().body(site)
}

#[get("/buzzer")]
async fn buzzer() -> impl Responder {
	let site = format!("{}<form action=\"/buzz\">
			<input type=\"submit\" class=\"buzzer\" value=\"Buzzer!\">
		</form>{}", SITE_HEADER, SITE_FOOTER);
	let site = site.into_bytes();
	HttpResponse::Ok().body(site)
}
