from django.urls import path

from . import views

urlpatterns = [
    path("health", views.health),  # ignored by default
    path("projects/<str:project_id>/tasks", views.tasks),
    path("projects/<str:project_id>/export", views.export),
]
