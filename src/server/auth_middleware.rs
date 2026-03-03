use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use std::rc::Rc;
use actix_web::body::EitherBody;
use crate::auth::{self, JwtConfig};

pub struct AuthMiddleware {
    pub jwt_config: JwtConfig,
}

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;    type Error = Error;
    type InitError = ();
    type Transform = AuthMiddlewareInner<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(AuthMiddlewareInner {
            service: Rc::new(service),
            jwt_config: self.jwt_config.clone(),
        })
    }
}

pub struct AuthMiddlewareInner<S> {
    service: Rc<S>,
    jwt_config: JwtConfig,
}

impl<S, B> Service<ServiceRequest> for AuthMiddlewareInner<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &self,
        ctx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let path = req.path().to_string();

        // Permettiamo sempre /auth
        if path == "/auth" {
            let fut: <S as Service<ServiceRequest>>::Future = self.service.call(req);
            return Box::pin(async move {
                let res = fut.await?;
                Ok(res.map_into_left_body())
            });      
        }

        let auth_header = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");

        let token = auth_header.strip_prefix("Bearer ").unwrap_or("");

        if token.is_empty() || !auth::verify_jwt(token, &self.jwt_config) {
            let response = req.into_response(HttpResponse::Unauthorized().finish());
            return Box::pin(async move {
                Ok(response.map_into_right_body())
            });        
        }

        let fut = self.service.call(req);
        Box::pin(async move {
            let res = fut.await?;
            Ok(res.map_into_left_body())
        })
    }
}